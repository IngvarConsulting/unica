use super::{write, RealReaderFixture, ViewRequest};
use unica_workspace::test_support::tree_snapshot;

const BODY: &str = "main:CommonModule.РеактивныйСервер.Body";

#[test]
// rule: INV.SOURCE.MODULE-COMPILATION-AND-BODY
fn body_ranges_beyond_eof_are_refused_for_broken_and_healthy_sources() {
    let fixture = RealReaderFixture::new();
    for text in ["Procedure Broken(\n", "// healthy\n"] {
        write(
            &fixture
                .source
                .join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
            text,
        );
        for range in ["#L999", "#L1-L2"] {
            let result = fixture
                .view_service()
                .view(ViewRequest::new(&format!("{BODY}{range}")).unwrap());
            assert!(!result.ok, "{range}: {result:?}");
            assert_eq!(result.diagnostics[0]["code"], "bad_value", "{result:?}");
        }
    }
}

#[test]
// rule: INV.SOURCE.BSL-SYNTAX-ERROR-READING
fn broken_bsl_body_keeps_exact_text_and_localizes_the_error() {
    let fixture = RealReaderFixture::new();
    let text = "// retained\r\nProcedure Broken(\r\n";
    write(
        &fixture
            .source
            .join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        text,
    );
    let before = tree_snapshot(&fixture.source);
    let result = fixture.view_service().view(ViewRequest::new(BODY).unwrap());
    assert!(result.ok, "{result:?}");
    assert_eq!(result.source_text.as_deref(), Some(text));
    assert_eq!(
        result.rev.as_deref(),
        Some(unica_source_contract::read::module_text_revision(text.as_bytes()).as_str())
    );
    let diagnostic = result
        .diagnostics
        .first()
        .expect("broken BSL must report syntax diagnostics beside its text");
    assert_eq!(diagnostic["code"], "bsl-parse-error");
    assert!(diagnostic["startLine"].as_u64().unwrap() >= 2);
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
// rule: INV.SOURCE.BSL-SYNTAX-ERROR-READING
fn broken_bsl_structure_refuses_and_routes_back_to_body() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        "Procedure Broken(\n",
    );
    for suffix in ["Method", "Event"] {
        let result = fixture.view_service().view(
            ViewRequest::new(&format!("main:CommonModule.РеактивныйСервер.{suffix}")).unwrap(),
        );
        assert!(!result.ok, "{result:?}");
        assert!(result.data.is_none());
        assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
        assert!(result.next.iter().any(|next| next["args"]["at"] == BODY));
    }
}

#[test]
// rule: INV.SOURCE.BSL-SYNTAX-ERROR-READING
fn broken_bsl_range_uses_full_source_coordinates_and_reports_the_unshown_count() {
    let fixture = RealReaderFixture::new();
    let text = format!(
        "// selected\nProcedure Broken(\n{}\n{}",
        "ОченьДлинноеИмя".repeat(1000),
        "If Then\n".repeat(300)
    );
    write(
        &fixture
            .source
            .join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        &text,
    );
    let result = fixture
        .view_service()
        .view(ViewRequest::new(&format!("{BODY}#L1")).unwrap());
    assert!(result.ok, "{result:?}");
    assert_eq!(result.source_text.as_deref(), Some("// selected\n"));
    assert_eq!(
        result.rev.as_deref(),
        Some(unica_source_contract::read::module_text_revision(text.as_bytes()).as_str())
    );
    assert_eq!(result.diagnostics.len(), 1);
    let diagnostic = &result.diagnostics[0];
    assert!(diagnostic["startLine"].as_u64().unwrap() > 1);
    assert!(
        diagnostic["totalSyntaxErrors"].as_u64().unwrap() > diagnostic["shown"].as_u64().unwrap()
    );
    assert!(serde_json::to_vec(&result).unwrap().len() < unica_bootstrap::DEFAULT_RESPONSE_BUDGET);
}

#[test]
// rule: INV.SOURCE.BSL-SYNTAX-ERROR-READING
fn oversized_bsl_range_remains_readable_with_explicit_unknown_syntax() {
    let fixture = RealReaderFixture::new();
    let text = format!(
        "// selected\n{}",
        "// padding\n".repeat(crate::application::v13::source_text::MAX_SOURCE_TEXT_BYTES)
    );
    write(
        &fixture
            .source
            .join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        &text,
    );
    let result = fixture
        .view_service()
        .view(ViewRequest::new(&format!("{BODY}#L1")).unwrap());
    assert!(result.ok, "{result:?}");
    assert_eq!(result.source_text.as_deref(), Some("// selected\n"));
    assert_eq!(
        result.rev.as_deref(),
        Some(unica_source_contract::read::module_text_revision(text.as_bytes()).as_str())
    );
    assert!(result.diagnostics.is_empty());
    assert!(!result.data.unwrap()["limits"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
// rule: INV.SOURCE.BSL-SYNTAX-ERROR-READING
fn body_syntax_uses_the_editor_buffer_and_preserves_the_healthy_disk() {
    use super::*;
    use unica_workspace::documents::{DocumentOverlay, DocumentText};
    let fixture = RealReaderFixture::new();
    let relative = Path::new("CommonModules/РеактивныйСервер/Ext/Module.bsl");
    write(
        &fixture.source.join(relative),
        "Procedure Healthy()\nEndProcedure\n",
    );
    let before = tree_snapshot(&fixture.source);
    let overlay = DocumentOverlay::default();
    overlay
        .put(
            "editor",
            "main",
            relative,
            DocumentText {
                version: 1,
                text: Arc::from("// buffer\nProcedure Broken(\n"),
            },
        )
        .unwrap();
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let read = ProviderReadAuthority::new(
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        root,
    )
    .with_documents(overlay.session_documents("editor", "main").unwrap());
    let authority = LogicalViewReadAuthority::with_read_authority(
        &fixture.cancellation,
        read,
        PlatformProfile::v8_3_27(),
        ProviderDeadline::no_deadline(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());
    let result = service.view(ViewRequest::new(&format!("{BODY}#L1")).unwrap());
    assert!(result.ok, "{result:?}");
    assert_eq!(result.source_text.as_deref(), Some("// buffer\n"));
    assert_eq!(
        result.rev.as_deref(),
        Some(
            unica_source_contract::read::module_text_revision(b"// buffer\nProcedure Broken(\n")
                .as_str()
        )
    );
    assert_eq!(result.diagnostics[0]["code"], "bsl-parse-error");
    assert_eq!(tree_snapshot(&fixture.source), before);
    let healthy = fixture.view_service().view(ViewRequest::new(BODY).unwrap());
    assert!(healthy.ok && healthy.diagnostics.is_empty());
    assert!(
        fixture
            .view_service()
            .view(ViewRequest::new("main:CommonModule.РеактивныйСервер.Method.Healthy").unwrap())
            .ok
    );
}

#[test]
// rule: INV.SOURCE.BSL-SYNTAX-ERROR-READING
fn body_syntax_is_derived_from_its_retained_text_even_when_disk_changes_after_read() {
    use super::*;
    let fixture = RealReaderFixture::new();
    let module = fixture
        .source
        .join("CommonModules/РеактивныйСервер/Ext/Module.bsl");
    write(&module, "// observed\nProcedure Broken(\n");
    let authority = fixture.read_authority();
    let target = MetadataAddress::parse_logical("CommonModule.РеактивныйСервер.Module").unwrap();
    let body = QualifiedAddress::parse(&format!("{BODY}#L1")).unwrap();
    let (text, full) = authority
        .read
        .module_text_with_syntax_input(&target, body.line_range(), || Ok(()))
        .unwrap();
    write(&module, "// changed\nProcedure Healthy()\nEndProcedure\n");
    let module = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap();
    let observed = authority
        .body_syntax_observation(&module, text, full)
        .unwrap();
    assert_eq!(
        observed.text,
        crate::application::v13::source_text::ModuleText::Text {
            text: "// observed\n".into(),
            revision: unica_source_contract::read::module_text_revision(
                b"// observed\nProcedure Broken(\n"
            ),
        }
    );
    assert_eq!(observed.diagnostics[0]["code"], "bsl-parse-error");
}

#[test]
// rule: INV.SOURCE.BSL-SYNTAX-ERROR-READING
fn body_syntax_keeps_cancellation_deadline_and_source_read_failures() {
    use super::*;
    let fixture = RealReaderFixture::new();
    let target = MetadataAddress::parse_logical("CommonModule.РеактивныйСервер.Module").unwrap();
    let authority = fixture.read_authority();
    for code in [RefusalCode::Cancelled, RefusalCode::DeadlineExceeded] {
        let error = authority
            .read
            .module_text_with_syntax_input(&target, None, || {
                Err(ViewError::new(code, "interrupted"))
            })
            .unwrap_err();
        assert_eq!(error.code(), code);
    }
    write(
        &fixture
            .source
            .join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        "// initial\n",
    );
    std::fs::write(
        fixture
            .source
            .join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        [0xff],
    )
    .unwrap();
    let invalid = fixture.view_service().view(ViewRequest::new(BODY).unwrap());
    assert!(!invalid.ok && invalid.source_text.is_none());
}
