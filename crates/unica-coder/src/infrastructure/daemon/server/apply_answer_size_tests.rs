//! #1119: the apply answer does not echo the request, so it does not grow
//! with the number of operations.
use super::actor_capacity_tests::{direct_v5, subsystem_picture_workspace};
use super::*;
use serde_json::{json, Value};
use unica_execution::invocation_store::ToolIdentity;

const LARGE_BATCH: usize = 800;

/// The cache report names the random per-workspace epoch and does not depend
/// on the request, so it is left out of the comparison.
fn answer_bytes(result: &DomainResult) -> usize {
    let mut result = result.clone();
    result.data.as_mut().unwrap()["cache"] = Value::Null;
    serde_json::to_vec(&result).unwrap().len()
}

/// Plans and publishes `count` edits of one subsystem; returns the request,
/// preview and publication sizes.
fn batch_sizes(count: usize) -> (usize, DomainResult, DomainResult) {
    let workspace = subsystem_picture_workspace("", false);
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let hint = std::fs::canonicalize(workspace.path()).unwrap();
    let call = |arguments: Value| {
        direct_v5(
            &runtime,
            InvocationRequest::new(
                ToolIdentity::Apply,
                arguments,
                hint.to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap()
    };
    // Equal-width values keep every answer field except the count the same size.
    let ops = (0..count)
        .map(|index| json!({"op": "props.set", "args": {"values": {"Comment": format!("batch {index:04}")}}}))
        .collect::<Vec<_>>();
    let request = json!({"at": "main:Subsystem.Sales", "ops": ops});
    let preview = call(request.clone());
    assert!(preview.ok, "{preview:?}");
    let token = preview.data.as_ref().unwrap()["executionToken"].clone();
    let published = call(json!({"executionToken": token}));
    assert!(published.ok, "{published:?}");
    (
        serde_json::to_vec(&request).unwrap().len(),
        preview,
        published,
    )
}

#[test]
fn apply_answer_does_not_grow_with_the_number_of_operations() {
    let (single_request, single_preview, single_published) = batch_sizes(1);
    let (large_request, large_preview, large_published) = batch_sizes(LARGE_BATCH);
    assert!(
        large_request > 40 * single_request,
        "the batch must be large"
    );
    for (single, large) in [
        (&single_preview, &large_preview),
        (&single_published, &large_published),
    ] {
        let data = large.data.as_ref().unwrap();
        assert_eq!(data["operationCount"], LARGE_BATCH, "{data}");
        assert_eq!(
            data["cache"]["events"],
            single.data.as_ref().unwrap()["cache"]["events"]
        );
        assert_eq!(single.data.as_ref().unwrap()["operationCount"], 1);
        assert!(data.get("operations").is_none(), "{data}");
        assert_eq!(large.changed, single.changed);
        // The token, the plan hash and the count differ in value, not size;
        // the count gains two digits.
        let (single, large) = (answer_bytes(single), answer_bytes(large));
        assert!(
            large <= single + 2,
            "a {LARGE_BATCH}-operation answer is {large} bytes against {single} for one"
        );
    }
}

#[test]
fn apply_answer_names_only_the_operation_that_warns() {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("src");
    std::fs::create_dir_all(source.join("Documents")).unwrap();
    std::fs::create_dir_all(source.join("Catalogs/Items/Ext")).unwrap();
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Items</Catalog><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Documents/Order.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Catalogs/Items.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>Items</Name><Synonym/><Comment/></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Catalogs/Items/Ext/ObjectModule.bsl"),
        "Процедура Заполнить()\n\tДокумент = Документы.Order.СоздатьДокумент();\nКонецПроцедуры\n",
    )
    .unwrap();
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let preview = direct_v5(
        &runtime,
        InvocationRequest::new(
            ToolIdentity::Apply,
            json!({"at": "main:Document.Order", "ops": [{"op": "object.remove", "args": {}}]}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(preview.ok, "{preview:?}");
    assert!(
        preview
            .warnings
            .iter()
            .any(|warning| warning["code"] == "code_references"),
        "{preview:?}"
    );
    let data = preview.data.as_ref().unwrap();
    assert_eq!(data["operationCount"], 1, "{data}");
    assert_eq!(
        data["operations"],
        json!([{"index": 0, "op": "object.remove", "at": "main:Document.Order"}]),
        "{data}"
    );
}
