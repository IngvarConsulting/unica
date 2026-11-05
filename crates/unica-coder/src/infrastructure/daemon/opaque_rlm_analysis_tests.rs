use super::*;

/// rule: INV.APP.PROVIDER-PRIVATE-STATE
#[test]
fn canonical_analysis_uses_supported_mcp_with_opaque_format_a() {
    check_opaque_format(0);
}
/// rule: INV.APP.PROVIDER-PRIVATE-STATE
#[test]
fn canonical_analysis_uses_supported_mcp_with_opaque_format_b() {
    check_opaque_format(1);
}
fn check_opaque_format(format: u8) {
    let workspace = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(workspace.path()).unwrap();
    let source = root.join("src");
    std::fs::create_dir_all(source.join("CommonModules/Probe/Ext")).unwrap();
    std::fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(source.join("Configuration.xml"),r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Opaque</Name></Properties><ChildObjects><CommonModule>Probe</CommonModule></ChildObjects></Configuration></MetaDataObject>"#).unwrap();
    std::fs::write(source.join("CommonModules/Probe.xml"),r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Probe</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#).unwrap();
    std::fs::write(
        source.join("CommonModules/Probe/Ext/Module.bsl"),
        "Procedure Alpha() Export\nEndProcedure\n\nProcedure Beta() Export\nEndProcedure\n",
    )
    .unwrap();
    let service = Arc::new(
        super::super::super::v13_service::CanonicalV13ReadService::with_search_providers_for_test(
            vec![Arc::new(
                crate::infrastructure::analysis_composition::rlm_provider(),
            )],
        ),
    );
    let runtime = V5CanonicalInvocationRuntime::new(service, Arc::new(TokioClock));
    let viewed = submit_canonical(
        &runtime,
        &root,
        ToolIdentity::View,
        serde_json::json!({"at":"main:CommonModule.Probe"}),
    );
    assert!(viewed.ok, "{viewed:?}");
    let context = unica_source_contract::workspace::WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    crate::infrastructure::workspace_services::with_opaque_rlm_for_test(&context, format, || {
        let request=InvocationRequest::new(ToolIdentity::Search,serde_json::json!({"query":"opaque-index-query","role":"semantic","scope":"main:Configuration"}),root.to_string_lossy(),7000).unwrap();
        let bound = runtime.bind(request).unwrap();
        let prepared = bound.prepare().unwrap();
        let result = prepared.execute(CancellationToken::new()).unwrap();
        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        let sections = data["matches"].as_array().unwrap();
        assert_eq!(sections.len(), 1, "{data:#}");
        let hits = sections[0]["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1, "{data:#}");
        let name = if format == 0 { "Alpha" } else { "Beta" };
        assert_eq!(hits[0]["symbol"], name, "{data:#}");
        assert_eq!(hits[0]["snippet"], name, "{data:#}");
        assert_eq!(
            hits[0]["at"],
            format!(
                "main:CommonModule.Probe.Method.{name}.Body#L{}",
                if format == 0 { 1 } else { 4 }
            ),
            "{data:#}"
        );
        let text = serde_json::to_string(&result.data).unwrap();
        let (present, absent) = if format == 0 {
            ("Alpha", "Beta")
        } else {
            ("Beta", "Alpha")
        };
        assert!(text.contains(present), "{text}");
        assert!(!text.contains(absent), "{text}");
        assert!(
            !text.contains("PRIVATE-A") && !text.contains("private-layout-v2"),
            "{text}"
        );
    });
}
