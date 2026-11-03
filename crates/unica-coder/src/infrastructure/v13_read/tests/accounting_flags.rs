use super::*;

const OWNER: &str = "main:ChartOfAccounts.Accounts";
const FLAGS: &[(&str, &[&str])] = &[
    (
        "AccountingFlag",
        &[
            "Валютный",
            "Количественный",
            "УчетПоПодразделениям",
            "НалоговыйУчет",
        ],
    ),
    (
        "ExtDimensionAccountingFlag",
        &["Суммовой", "Валютный", "Количественный"],
    ),
];

fn install(fixture: &RealReaderFixture) -> PathBuf {
    let path = fixture.source.join("ChartsOfAccounts/Accounts.xml");
    let xml = fs::read_to_string(&path).unwrap();
    let mut children = String::new();
    for (kind, names) in FLAGS {
        for name in *names {
            children.push_str(&format!(r#"<{kind}><Properties xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Name>{name}</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Признак {name}</v8:content></v8:item></Synonym><Comment>Definition {name}</Comment><Type><v8:Type>xs:boolean</v8:Type></Type></Properties></{kind}>"#));
        }
    }
    write(
        &path,
        &xml.replace(
            "<ChildObjects/>",
            &format!("<ChildObjects>{children}</ChildObjects>"),
        ),
    );
    path
}

#[test]
fn accounting_flag_collections_are_discoverable_and_preserve_all_definitions() {
    let fixture = RealReaderFixture::new();
    let path = install(&fixture);
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new(OWNER).unwrap());
    assert!(owner.ok);
    let branches = owner.data.as_ref().unwrap()["branches"].as_array().unwrap();
    for (kind, names) in FLAGS {
        let at = format!("{OWNER}.{kind}");
        let branch = branches
            .iter()
            .find(|branch| branch["at"] == at)
            .unwrap_or_else(|| panic!("missing {kind} branch"));
        assert_eq!(branch["count"], names.len());
        let mut request = ViewRequest::new(branch["at"].as_str().unwrap())
            .unwrap()
            .with_limit(2)
            .unwrap();
        let mut returned = Vec::new();
        loop {
            let page = service.view(request.clone());
            assert!(page.ok);
            for row in page.data.as_ref().unwrap()["items"].as_array().unwrap() {
                let target = row["at"].as_str().unwrap();
                let detail = service.view(ViewRequest::new(target).unwrap());
                assert!(detail.ok);
                assert_eq!(&detail.data.as_ref().unwrap()["props"], &row["props"]);
                assert_eq!(row["props"]["type"]["variants"][0]["kind"], "boolean");
                let name = target.rsplit('.').next().unwrap();
                assert_eq!(row["props"]["synonym"], format!("Признак {name}"));
                assert_eq!(row["props"]["comment"], format!("Definition {name}"));
                returned.push(target.to_string());
            }
            match page.cursor {
                Some(cursor) => request = request.with_cursor(cursor),
                None => break,
            }
        }
        assert_eq!(
            returned,
            names
                .iter()
                .map(|name| format!("{at}.{name}"))
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn accounting_flags_distinguish_empty_missing_and_inapplicable_collections() {
    for missing in [false, true] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("ChartsOfAccounts/Accounts.xml");
        if missing {
            write(
                &path,
                &fs::read_to_string(&path)
                    .unwrap()
                    .replace("<ChildObjects/>", ""),
            );
        }
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new(OWNER).unwrap());
        assert!(result.ok);
        let node = result.data.as_ref().unwrap();
        for (kind, _) in FLAGS {
            let at = format!("{OWNER}.{kind}");
            let branch = node["branches"]
                .as_array()
                .unwrap()
                .iter()
                .find(|branch| branch["at"] == at);
            let collection = service.view(ViewRequest::new(&at).unwrap());
            if missing {
                assert!(
                    branch.is_none(),
                    "unknown membership must not promise count zero"
                );
                assert!(node["limits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|limit| limit.as_str().is_some_and(|text| text.contains(kind))));
                assert!(!collection.ok);
                assert_eq!(collection.diagnostics[0]["detailCode"], "source_unreadable");
            } else {
                assert_eq!(branch.unwrap()["count"], 0);
                assert!(collection.ok);
                assert_eq!(collection.data.as_ref().unwrap()["items"], json!([]));
            }
        }
    }
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let catalog = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(catalog.ok);
    for (kind, _) in FLAGS {
        assert!(!catalog.data.as_ref().unwrap()["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|branch| branch["at"].as_str().is_some_and(|at| at.ends_with(kind))));
        let read = service.view(ViewRequest::new(&format!("main:Catalog.Items.{kind}")).unwrap());
        assert!(!read.ok);
        assert_eq!(read.diagnostics[0]["code"], "not_found");
    }
}

#[test]
fn accounting_flag_identity_damage_does_not_claim_empty_or_hide_the_other_collection() {
    for (kind, names) in FLAGS {
        for raw in [
            format!("<{kind} xmlns=\"urn:foreign\"><Properties><Name>Ghost</Name></Properties></{kind}>"),
            format!(r#"<{kind}><Properties xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Name>{}</Name><Type><v8:Type>xs:boolean</v8:Type></Type></Properties></{kind}>"#, names[0]),
            format!(r#"<{kind}><Properties xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Name>{}</Name><Type><v8:Type>xs:boolean</v8:Type></Type></Properties></{kind}>"#, names[0].to_lowercase()),
            format!(r#"<{kind}><Properties xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Name>{}</Name><Type><v8:Type>xs:boolean</v8:Type></Type></Properties></{kind}>"#, format_args!(" {} ", names[0])),
            format!("<{kind}><Properties/></{kind}>"),
            format!("<{kind}><Properties><Name>Broken</Name><Type><Unexpected/></Type></Properties></{kind}>"),
        ] {
            let fixture = RealReaderFixture::new();
            let path = install(&fixture);
            let xml = fs::read_to_string(&path).unwrap();
            write(&path, &xml.replace("</ChildObjects>", &format!("{raw}</ChildObjects>")));
            let service = fixture.view_service();
            let result = service.view(ViewRequest::new(OWNER).unwrap());
            assert!(result.ok);
            let owner = result.data.as_ref().unwrap();
            assert!(owner["branches"].as_array().unwrap().iter().all(|branch| branch["at"] != format!("{OWNER}.{kind}")));
            assert!(owner["limits"].as_array().unwrap().iter().any(|limit| limit.as_str().is_some_and(|text| text.contains(kind))));
            let broken = service.view(ViewRequest::new(&format!("{OWNER}.{kind}")).unwrap());
            assert!(!broken.ok);
            assert_eq!(broken.diagnostics[0]["detailCode"], "source_unreadable");
            let healthy = if *kind == "AccountingFlag" { "ExtDimensionAccountingFlag" } else { "AccountingFlag" };
            let read = service.view(ViewRequest::new(&format!("{OWNER}.{healthy}")).unwrap());
            assert!(read.ok);
            assert!(!read.data.as_ref().unwrap()["items"].as_array().unwrap().is_empty());
            let standard = service.view(ViewRequest::new(&format!("{OWNER}.StandardTabularSection.ExtDimensionTypes")).unwrap());
            assert!(standard.ok);
        }
    }
}

#[test]
fn accounting_flags_preserve_read_only_types_and_do_not_become_metadata_roots() {
    let fixture = RealReaderFixture::new();
    let path = install(&fixture);
    let xml = fs::read_to_string(&path).unwrap();
    write(
        &path,
        &xml.replacen(
            "<v8:Type>xs:boolean</v8:Type>",
            "<v8:TypeId>41dbb66b-ff77-4b8f-aaf8-0ca2012d3a6f</v8:TypeId>",
            1,
        ),
    );
    let result = fixture
        .view_service()
        .view(ViewRequest::new(&format!("{OWNER}.AccountingFlag.Валютный")).unwrap());
    assert!(result.ok);
    let observed = &result.data.as_ref().unwrap()["props"]["type"];
    assert_eq!(
        observed["variants"][0],
        json!({"kind":"typeId","typeId":"41dbb66b-ff77-4b8f-aaf8-0ca2012d3a6f"})
    );
    assert_eq!(observed["mutationCapability"], "readOnly");
    for (kind, _) in FLAGS {
        let at = format!("main:{kind}.Валютный");
        let parsed = QualifiedAddress::parse(&at).unwrap();
        assert!(!parsed.segments()[0].kind().is_metadata_kind());
        let read = fixture.view_service().view(ViewRequest::new(&at).unwrap());
        assert!(
            !read.ok,
            "a member does not become a source-set metadata root"
        );
    }
}

#[test]
fn accounting_flags_read_the_retained_native_empty_chart_without_changing_bytes() {
    let fixture = RealReaderFixture::new();
    let native = fixture_path("platform_8_3_27/new-xml/native-chart-of-accounts-descriptor.xml");
    let bytes = fs::read(native).unwrap();
    let path = fixture.source.join("ChartsOfAccounts/CorpusAccounts.xml");
    fs::write(&path, &bytes).unwrap();
    let config = fixture.source.join("Configuration.xml");
    write(
        &config,
        &fs::read_to_string(&config).unwrap().replace(
            "</ChildObjects>",
            "<ChartOfAccounts>CorpusAccounts</ChartOfAccounts></ChildObjects>",
        ),
    );
    let service = fixture.view_service();
    let owner = "main:ChartOfAccounts.CorpusAccounts";
    let result = service.view(ViewRequest::new(owner).unwrap());
    assert!(result.ok);
    let branches = result.data.as_ref().unwrap()["branches"]
        .as_array()
        .unwrap();
    for (kind, _) in FLAGS {
        let at = format!("{owner}.{kind}");
        assert_eq!(
            branches.iter().find(|branch| branch["at"] == at).unwrap()["count"],
            0
        );
        let read = service.view(ViewRequest::new(&at).unwrap());
        assert!(read.ok);
        assert_eq!(read.data.as_ref().unwrap()["items"], json!([]));
    }
    assert_eq!(fs::read(path).unwrap(), bytes);
}
