use super::*;

#[test]
fn canonical_service_descriptive_findings_keep_failed_verdict_and_readable_members() {
    for (file, owner, member, field, tag, replacement, code, tail) in [
        (
            "HTTPServices/ПроверкаСвязи.xml",
            "HTTPService.ПроверкаСвязи",
            "Пинг",
            "details.urlTemplates[0].comment",
            "Comment",
            "<Comment><Hidden/></Comment>",
            "validation_failed",
            "URLTemplate.Пинг",
        ),
        (
            "HTTPServices/ПроверкаСвязи.xml",
            "HTTPService.ПроверкаСвязи",
            "GET",
            "details.urlTemplates[0].methods[0].synonym",
            "Synonym",
            "<Synonym xmlns=\"urn:foreign\"/>",
            "provider_unavailable",
            "URLTemplate.Пинг.Method.GET",
        ),
        (
            "WebServices/ОбменДанными.xml",
            "WebService.ОбменДанными",
            "Пинг",
            "details.operations[0].dataLockControlMode",
            "DataLockControlMode",
            "<DataLockControlMode>AutomaticAndManaged</DataLockControlMode>",
            "validation_failed",
            "Operation.Пинг",
        ),
        (
            "WebServices/ОбменДанными.xml",
            "WebService.ОбменДанными",
            "Отправитель",
            "details.operations[1].parameters[0].comment",
            "Comment",
            "<Comment/><Comment/>",
            "provider_unavailable",
            "Operation.ПринятьПакет.Parameter.Отправитель",
        ),
    ] {
        let fixture = Fixture::new("workspace");
        let runtime = runtime();
        let before = check(&runtime, &fixture, &format!("main:{owner}"));
        assert!(
            !before.1.iter().any(|d| d["field"] == field),
            "{:?}",
            before
        );
        let path = fixture.source.join(file);
        let xml = std::fs::read_to_string(&path).unwrap();
        let from = xml.find(&format!("<Name>{member}</Name>")).unwrap();
        let start = from + xml[from..].find(&format!("<{tag}")).unwrap();
        let relative_end = xml[start..].find('>').unwrap() + 1;
        let end = if xml[start..start + relative_end].ends_with("/>") {
            start + relative_end
        } else {
            start + xml[start..].find(&format!("</{tag}>")).unwrap() + tag.len() + 3
        };
        std::fs::write(
            &path,
            format!("{}{replacement}{}", &xml[..start], &xml[end..]),
        )
        .unwrap();
        let image = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let (status, diagnostics) = check(&runtime, &fixture, &format!("main:{owner}"));
        assert_eq!(status, "failed", "{field}: {diagnostics:?}");
        let found = finding(&diagnostics, code, field);
        assert_eq!(found["metadataPath"], owner);
        assert_eq!(found["severity"], "error");
        assert!(!found["message"].as_str().unwrap().is_empty());
        let node = view(&runtime, &fixture, &format!("main:{owner}.{tail}"));
        assert!(limits(&node).iter().any(|s| s.contains(field)), "{node:#}");
        assert!(node["props"].as_object().is_some_and(|p| !p.is_empty()));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            image
        );
    }
}
