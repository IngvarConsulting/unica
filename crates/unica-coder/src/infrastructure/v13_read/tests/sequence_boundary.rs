use super::*;

#[test]
fn sequence_boundary_view_preserves_absence_and_localizes_damage() {
    for (raw, expected, damaged) in [
        ("<MoveBoundaryOnPosting>DontMove</MoveBoundaryOnPosting>", Some("DontMove"), false),
        ("<MoveBoundaryOnPosting>Move</MoveBoundaryOnPosting>", Some("Move"), false),
        ("", None, false),
        ("<MoveBoundaryOnPosting/>", None, true),
        ("<MoveBoundaryOnPosting>Unknown</MoveBoundaryOnPosting>", None, true),
        ("<MoveBoundaryOnPosting xmlns=\"urn:foreign\">Move</MoveBoundaryOnPosting>", None, true),
        ("<MoveBoundaryOnPosting><Value>Move</Value></MoveBoundaryOnPosting>", None, true),
        ("<MoveBoundaryOnPosting>Move</MoveBoundaryOnPosting><MoveBoundaryOnPosting>DontMove</MoveBoundaryOnPosting>", None, true),
    ] {
        let fixture = RealReaderFixture::new();
        let configuration = fixture.source.join("Configuration.xml");
        write(&configuration, &fs::read_to_string(&configuration).unwrap().replace("</ChildObjects>", "<Sequence>Sales</Sequence></ChildObjects>"));
        let path = fixture.source.join("Sequences/Sales.xml");
        write(&path, &format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Sequence uuid="16770000-0000-4000-8000-000000000001"><Properties><Name>Sales</Name><Comment>retained</Comment>{raw}</Properties></Sequence></MetaDataObject>"#));
        let before = fs::read(&path).unwrap();
        let result = fixture.view_service().view(ViewRequest::new("main:Sequence.Sales").unwrap());
        assert!(result.ok, "{result:?}");
        let node = result.data.as_ref().unwrap();
        assert_eq!(node["props"].get("MoveBoundaryOnPosting").and_then(serde_json::Value::as_str), expected);
        assert_eq!(node["at"], "main:Sequence.Sales");
        assert_eq!(node["props"]["kind"], "Sequence");
        assert_eq!(node.get("limits").is_some_and(|v| v.as_array().is_some_and(|items| items.iter().any(|s| s.as_str().is_some_and(|s| s.contains("MoveBoundaryOnPosting"))))), damaged);
        assert_eq!(fs::read(path).unwrap(), before);
    }
}
