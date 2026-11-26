use super::*;
use unica_code_intelligence::model::{CodeIntelligenceContext, ProviderSearchSection};
use unica_source_contract::workspace::ResolvedSourceRoot;

const MODULE: &str = "CommonModules/РеактивныйСервер/Ext/Module.bsl";
const SHOWN: &str = "&После(\"Original\")\nПроцедура Probe()\n    Value = 1;";
const TAIL: &str = "    Value = 2;\n    Value = 1;\nКонецПроцедуры";

fn search_result(fixture: &RealReaderFixture, reply: &str) -> ProviderSearchSection {
    let context = CodeIntelligenceContext::new(
        fixture.context.clone(),
        ResolvedSourceRoot {
            source_set: Some("main".to_string()),
            path: fixture.source.clone(),
        },
        Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
            fixture.root.path(),
        )),
    );
    unica_adapter_bsl_analyzer::provider::parse_bsl_analyzer_search(
        reply,
        &context,
        20,
        &fixture.cancellation,
        crate::infrastructure::analysis_composition::source_bindings(),
    )
}

fn listing(range: &str, shown: &str, marker: &str) -> String {
    let mut listing = format!("#1 [L] {MODULE}:{range} :: Probe (procedure)\n");
    for line in shown.split('\n').chain(std::iter::once(marker)) {
        listing.push_str(&format!("  │ {line}\n"));
    }
    listing
}

/// rule: INV.APP.CODE-SEARCH-HIT-ADDRESS
#[test]
fn analyzer_snippet_truncation_proves_only_the_unique_displayed_lines() {
    for eol in ["\n", "\r\n"] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join(MODULE);
        let source = format!("// prelude\n{SHOWN}\n{TAIL}\n").replace('\n', eol);
        fs::write(&path, &source).unwrap();
        let module = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap();
        let at = format!("{module}.Method.Probe.Body#L2-L4");
        let authority = fixture.operation_read_authority();
        let marker = "... (3 more lines)";
        let listing_cases = [
            ("2-7", SHOWN.to_string(), marker, Some(at.as_str())),
            ("2-7", SHOWN.replace("    Value", "\tValue"), marker, None),
            ("3-7", SHOWN.to_string(), "... (2 more lines)", None),
            ("2-7", SHOWN.to_string(), "... (2 more lines)", None),
            ("2-8", SHOWN.to_string(), "... (4 more lines)", None),
            ("4-7", "    Value = 1;".to_string(), marker, None),
            ("2", SHOWN.to_string(), marker, None),
            ("2-7", SHOWN.to_string(), "... (oops more lines)", None),
            ("2-7", " ".to_string(), "... (5 more lines)", None),
        ];
        for (range, shown, marker, expected) in listing_cases {
            let section = search_result(&fixture, &listing(range, &shown, marker));
            assert!(section.search_complete, "{section:?}");
            let hit = &section.hits[0];
            assert_eq!(hit.rank, Some(1));
            assert_eq!(hit.snippet, format!("{shown}\n{marker}"));
            let addresses = authority.bodies_for_search_hits(&module, [hit]).unwrap();
            assert_eq!(addresses[0].as_deref(), expected, "{hit:?}");
        }

        let full = format!("{SHOWN}\n{TAIL}");
        let full_at = format!("{module}.Method.Probe.Body#L2-L7");
        let mut hit =
            json!({"location":{"path":MODULE},"line_start":2,"line_end":7,"snippet":full});
        let mut structured = vec![(hit.clone(), Some(full_at.as_str()))];
        hit["snippet"] = json!(SHOWN);
        for count in [json!(3), json!(2), json!(0), json!(null), json!("3")] {
            let expected = (count == 3).then_some(at.as_str());
            hit["snippet_truncated_lines"] = count;
            structured.push((hit.clone(), expected));
        }
        for (raw, expected) in structured {
            let section = search_result(&fixture, &json!({"hits":[raw]}).to_string());
            let hit = &section.hits[0];
            assert_eq!(hit.snippet, raw["snippet"]);
            assert_eq!(
                serde_json::to_value(hit).unwrap()["snippet"],
                raw["snippet"]
            );
            assert!(serde_json::to_value(hit)
                .unwrap()
                .get("snippetEvidence")
                .is_none());
            let addresses = authority.bodies_for_search_hits(&module, [hit]).unwrap();
            assert_eq!(addresses[0].as_deref(), expected, "{raw:#}");
        }

        let service = fixture.view_service();
        let opened = service.view(ViewRequest::new(&at).unwrap());
        assert_eq!(
            opened.source_text.unwrap(),
            format!("{SHOWN}\n").replace('\n', eol)
        );
        assert_eq!(fs::read_to_string(path).unwrap(), source);
    }
}

/// rule: INV.APP.CODE-SEARCH-HIT-ADDRESS
#[test]
fn successful_proof_rechecks_source_bytes_even_with_a_cached_projection() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join(MODULE);
    let source = format!("// prelude\n{SHOWN}\n{TAIL}\n");
    fs::write(&path, &source).unwrap();
    let module = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap();
    let section = search_result(&fixture, &listing("2-7", SHOWN, "... (3 more lines)"));
    let authority = fixture.operation_read_authority();
    assert_eq!(authority.module_text(&module).unwrap(), source);
    let replacement = source.replacen("Value = 1", "Value = 9", 1);
    let mut pending = Some((path, replacement));
    review_set_revision_identity_hooks(
        || {},
        move || {
            if let Some((path, replacement)) = pending.take() {
                fs::write(path, replacement).unwrap();
            }
        },
    );
    let result = authority.bodies_for_search_hits(&module, &section.hits);
    review_clear_revision_identity_hooks();
    assert_eq!(result.unwrap_err().code(), RefusalCode::ConcurrentChange);
    let fresh = fixture.operation_read_authority();
    assert_eq!(
        fresh
            .bodies_for_search_hits(&module, &section.hits)
            .unwrap(),
        vec![None]
    );
}
