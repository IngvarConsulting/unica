use super::*;

const DOCS: [&str; 5] = ["Sale", "Return", "GiftSale", "GiftReturn", "Correction"];
const COLUMN: &str = "main:DocumentJournal.Receipts.Column.Status";
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

fn descriptor(kind: &str, name: &str, properties: &str, children: &str) -> String {
    format!("<MetaDataObject xmlns=\"{MD}\" xmlns:xr=\"{XR}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" version=\"2.20\"><{kind} uuid=\"15820000-0000-4000-8000-000000000001\"><Properties><Name>{name}</Name>{properties}</Properties><ChildObjects>{children}</ChildObjects></{kind}></MetaDataObject>")
}

fn install(fixture: &RealReaderFixture, references: &str) {
    let root = fixture.source.join("Configuration.xml");
    let registrations = DOCS
        .iter()
        .map(|name| format!("<Document>{name}</Document>"))
        .collect::<String>();
    write(
        &root,
        &fs::read_to_string(&root).unwrap().replace(
            "</ChildObjects>",
            &format!("{registrations}<DocumentJournal>Receipts</DocumentJournal></ChildObjects>"),
        ),
    );
    for name in DOCS {
        write(&fixture.source.join(format!("Documents/{name}.xml")),&descriptor("Document",name,"","<Attribute uuid=\"15820000-0000-4000-8000-000000000002\"><Properties><Name>Status</Name><Type><v8:Type>xs:string</v8:Type></Type></Properties></Attribute>"));
    }
    let registered = DOCS
        .iter()
        .map(|name| format!("<xr:Item xsi:type=\"xr:MDObjectRef\">Document.{name}</xr:Item>"))
        .collect::<String>();
    let mut columns=(0..8).map(|i|format!("<Column uuid=\"15820000-0000-4000-8000-000000000003\"><Properties><Name>Other{i}</Name><References/></Properties></Column>")).collect::<String>();
    columns.push_str(&format!("<Column uuid=\"15820000-0000-4000-8000-000000000004\"><Properties><Name>Status</Name>{references}</Properties></Column>"));
    write(
        &fixture.source.join("DocumentJournals/Receipts.xml"),
        &descriptor(
            "DocumentJournal",
            "Receipts",
            &format!("<RegisteredDocuments>{registered}</RegisteredDocuments>"),
            &columns,
        ),
    );
}

fn references() -> String {
    format!(
        "<References>{}</References>",
        DOCS.iter()
            .map(|name| format!(
                "<xr:Item xsi:type=\"xr:MDObjectRef\">Document.{name}.Attribute.Status</xr:Item>"
            ))
            .collect::<String>()
    )
}

#[test]
fn journal_column_references_keep_five_distinct_targets_and_read_the_returned_field() {
    let fixture = RealReaderFixture::new();
    install(&fixture, &references());
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new("main:DocumentJournal.Receipts").unwrap());
    assert!(owner.ok);
    assert!(owner.data.unwrap()["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at":"main:DocumentJournal.Receipts.Column","count":9})));
    let registered =
        service.view(ViewRequest::new("main:DocumentJournal.Receipts.Relation").unwrap());
    assert!(registered.ok);
    assert_eq!(
        registered.data.unwrap()["items"].as_array().unwrap().len(),
        5
    );
    let columns = service.view(ViewRequest::new("main:DocumentJournal.Receipts.Column").unwrap());
    assert!(columns.ok);
    let data = columns.data.unwrap();
    assert_eq!(data["items"].as_array().unwrap().len(), 9);
    let at = data["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["at"] == COLUMN)
        .unwrap()["at"]
        .as_str()
        .unwrap();
    let detail = service.view(ViewRequest::new(at).unwrap());
    assert!(detail.ok);
    let detail = detail.data.unwrap();
    let branch = json!({"at":format!("{COLUMN}.Relation"),"count":5});
    assert!(
        detail["branches"]
            .as_array()
            .is_some_and(|branches| branches.contains(&branch)),
        "{detail:#}"
    );
    let mut cursor = None;
    let mut items = Vec::new();
    loop {
        let mut request = ViewRequest::new(branch["at"].as_str().unwrap())
            .unwrap()
            .with_limit(2)
            .unwrap();
        if let Some(value) = cursor.take() {
            request = request.with_cursor(value);
        }
        let page = service.view(request);
        assert!(page.ok, "{:?}", refusal_codes(&page));
        items.extend(page.data.unwrap()["items"].as_array().unwrap().clone());
        cursor = page.cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(items,DOCS.iter().map(|name|json!({"relation":"reference","at":format!("main:Document.{name}.Attribute.Status"),"kind":"Attribute"})).collect::<Vec<_>>());
    for item in items {
        let field = service.view(ViewRequest::new(item["at"].as_str().unwrap()).unwrap());
        assert!(field.ok, "{:?}", refusal_codes(&field));
        assert_eq!(field.data.unwrap()["kind"], "Attribute");
    }
    let denied = service.view(ViewRequest::new(&format!("{COLUMN}.Relation.Sale")).unwrap());
    assert!(!denied.ok);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn column_reference_damage_preserves_independent_sources_and_distinguishes_empty() {
    for references in [
        "".to_string(),
        "<References/>".to_string(),
        "<References/><References/>".to_string(),
        "<References xmlns=\"urn:foreign\"/>".to_string(),
        references().replace(
            "Document.Sale.Attribute.Status",
            "Document.Sale.Attribute.Missing",
        ),
        references().replace(
            "Document.Sale.Attribute.Status",
            "Document.Missing.Attribute.Status",
        ),
        references().replacen("xr:MDObjectRef", "xr:Unknown", 1),
        references().replace(
            "Document.Sale.Attribute.Status",
            "Document.Sale.TabularSection.Status",
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, &references);
        let service = fixture.view_service();
        let relation = service.view(ViewRequest::new(&format!("{COLUMN}.Relation")).unwrap());
        assert!(relation.ok, "{:?}", refusal_codes(&relation));
        let data = relation.data.unwrap();
        let count = data["items"].as_array().unwrap().len();
        if references == "<References/>" {
            assert_eq!(count, 0);
            assert!(data.get("limits").is_none(), "{data:#}");
        } else {
            assert!(
                data["limits"]
                    .as_array()
                    .is_some_and(|limits| !limits.is_empty()),
                "{data:#}"
            );
            assert_eq!(
                count,
                if references.contains("<xr:Item") {
                    4
                } else {
                    0
                },
                "{data:#}"
            );
        }
        let owner =
            service.view(ViewRequest::new("main:DocumentJournal.Receipts.Relation").unwrap());
        assert!(owner.ok);
        assert_eq!(owner.data.unwrap()["items"].as_array().unwrap().len(), 5);
        let empty = service.view(
            ViewRequest::new("main:DocumentJournal.Receipts.Column.Other0.Relation").unwrap(),
        );
        assert!(empty.ok);
        assert_eq!(empty.data.unwrap()["items"], json!([]));
    }
}

#[test]
fn column_reference_identity_proof_rejects_ambiguous_fields_but_not_independent_type_damage() {
    for (from, to, expected) in [
        (
            "<Name>Status</Name>",
            "<Name>Status</Name><Name>Status</Name>",
            4,
        ),
        (
            "<Name>Status</Name>",
            "<Name><Nested>Status</Nested></Name>",
            4,
        ),
        (
            "<Name>Status</Name>",
            "<Name xmlns=\"urn:foreign\">Status</Name>",
            4,
        ),
        ("<Name>Status</Name>", "<Name>Other</Name>", 4),
        (
            "<v8:Type>xs:string</v8:Type>",
            "<v8:Type>xs:Unknown</v8:Type>",
            5,
        ),
        (
            "</Attribute>",
            "</Attribute><Attribute><Properties><Name>Status</Name></Properties></Attribute>",
            4,
        ),
        (
            "</Properties></Attribute>",
            "</Properties><Properties><Name>Status</Name></Properties></Attribute>",
            4,
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, &references());
        let path = fixture.source.join("Documents/Sale.xml");
        write(&path, &fs::read_to_string(&path).unwrap().replace(from, to));
        let service = fixture.view_service();
        let relation = service.view(ViewRequest::new(&format!("{COLUMN}.Relation")).unwrap());
        assert!(relation.ok, "{:?}", refusal_codes(&relation));
        let data = relation.data.unwrap();
        assert_eq!(
            data["items"].as_array().unwrap().len(),
            expected,
            "{from} -> {to}: {data:#}"
        );
        if expected == 4 {
            assert!(data["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()));
        }
    }
}
