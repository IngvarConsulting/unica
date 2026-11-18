use super::*;

#[test]
fn late_relation_failures_name_the_absolute_values_target_without_internal_wrappers() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"A"}}},
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"B"}}},
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"C"}}}
        ]}),
    );
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.A","ops":[
            {"op":"relation.add","args":{"values":{"relation":"owners","targets":[{"metadataPath":"Catalog.B"}]}}}
        ]}),
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    for (operation, target) in [
        ("relation.add", "Catalog.B"),
        ("relation.remove", "Catalog.C"),
    ] {
        let failure = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":"main:Catalog.A","ops":[
                {"op":"props.set","args":{"values":{"Comment":"must not publish"}}},
                {"op":operation,"args":{"values":{"relation":"owners","targets":[{"metadataPath":target}]}}}
            ]}),
        );
        assert!(!failure.ok, "late relation error must refuse");
        assert_eq!(failure.at.as_deref(), Some("ops[1].args.values.targets[0]"));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}
