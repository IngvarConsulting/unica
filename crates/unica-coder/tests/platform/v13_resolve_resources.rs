use super::*;

fn resource_resolve_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("resource resolve workspace");
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-resolve-resources"),
        root.path(),
    );
    root
}

fn resource_resolve_mcp(workspace: &std::path::Path) -> McpProcess {
    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc":"2.0", "id":1, "method":"initialize",
        "params":{"protocolVersion":"2025-06-18", "capabilities":{},
            "clientInfo":{"name":"resource-resolve", "version":"1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));
    mcp
}

#[test]
fn resolve_form_and_module_paths_preserve_the_exact_resource_on_public_mcp() {
    for source_kind in ["CONFIGURATION", "EXTENSION"] {
        let root = resource_resolve_fixture();
        let manifest = root.path().join("v8project.yaml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap()
            .replace("CONFIGURATION", source_kind);
        std::fs::write(manifest, text).unwrap();
        let mut mcp = resource_resolve_mcp(root.path());
        let mut id = 2;
        for (kind, name, form_prefix, form_address) in [
            (
                "DataProcessor",
                "Processor",
                "DataProcessors/Processor/Forms/Main",
                "main:DataProcessor.Processor.Form.Main",
            ),
            (
                "Document",
                "Invoice",
                "Documents/Invoice/Forms/Main",
                "main:Document.Invoice.Form.Main",
            ),
            (
                "CommonForm",
                "Main",
                "CommonForms/Main",
                "main:CommonForm.Main",
            ),
        ] {
            let mut resources = vec![
                (
                    format!("{form_prefix}/Ext/Form.xml"),
                    form_address.to_string(),
                    "Form",
                ),
                (
                    format!("{form_prefix}.xml"),
                    format!("{form_address}.Metadata"),
                    "Metadata",
                ),
                (
                    format!("{form_prefix}/Ext/Form/Module.bsl"),
                    format!("{form_address}.Module.Form"),
                    "Module",
                ),
            ];
            if kind != "CommonForm" {
                resources.extend([
                    (
                        format!("{kind}s/{name}/Ext/ObjectModule.bsl"),
                        format!("main:{kind}.{name}.Module.Object"),
                        "Module",
                    ),
                    (
                        format!("{kind}s/{name}/Ext/ManagerModule.bsl"),
                        format!("main:{kind}.{name}.Module.Manager"),
                        "Module",
                    ),
                ]);
            }
            for (path, address, expected_kind) in resources {
                for query in [
                    path.clone(),
                    format!("src/{path}"),
                    std::fs::canonicalize(root.path().join("src").join(&path))
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string(),
                ] {
                    let result = mcp.completed_tool_call(call_tool(
                        id,
                        "unica.resolve",
                        json!({"path":query}),
                    ));
                    id += 1;
                    assert_eq!(result["ok"], true, "{query}: {result:#}");
                    assert_eq!(result["data"]["at"], address);
                    assert_eq!(result["data"]["path"], path);
                    assert_eq!(result["data"]["kind"], expected_kind);
                    assert_eq!(result["data"]["lines"]["state"], "notLineBased");
                    let viewed =
                        mcp.completed_tool_call(call_tool(id, "unica.view", json!({"at":address})));
                    id += 1;
                    assert_eq!(viewed["ok"], true, "{viewed:#}");
                    let reverse = mcp.completed_tool_call(call_tool(
                        id,
                        "unica.resolve",
                        json!({"at": address}),
                    ));
                    id += 1;
                    assert_eq!(reverse["ok"], true, "{reverse:#}");
                    assert_eq!(reverse["data"]["at"], address);
                    assert_eq!(reverse["data"]["path"], path);
                    assert_eq!(reverse["data"]["kind"], expected_kind);
                }
            }
        }
        for (path, address) in [
            (
                "Documents/Invoice/Commands/Post/Ext/CommandModule.bsl",
                "main:Document.Invoice.Command.Post.Module.Command",
            ),
            ("Ext/SessionModule.bsl", "main:Module.Session"),
        ] {
            let result =
                mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"path":path})));
            id += 1;
            assert_eq!(result["ok"], true, "{path}: {result:#}");
            assert_eq!(result["data"]["at"], address);
            assert_eq!(result["data"]["path"], path);
            assert_eq!(result["data"]["kind"], "Module");
        }
        let descriptor = mcp.completed_tool_call(call_tool(
            id,
            "unica.resolve",
            json!({"at":"main:DataProcessor.Processor.Form.Main.Metadata"}),
        ));
        assert_eq!(descriptor["ok"], true, "{descriptor:#}");
        assert_eq!(
            descriptor["data"]["path"],
            "DataProcessors/Processor/Forms/Main.xml"
        );
        mcp.finish();
    }
}

#[test]
fn resolve_resources_refuse_unregistered_and_absent_targets_on_public_mcp() {
    let root = resource_resolve_fixture();
    std::fs::remove_file(
        root.path()
            .join("src/Documents/Invoice/Ext/ManagerModule.bsl"),
    )
    .unwrap();
    let mut mcp = resource_resolve_mcp(root.path());
    for (index, path) in [
        "CommonForms/Orphan.xml",
        "CommonForms/Orphan/Ext/Form.xml",
        "CommonForms/Orphan/Ext/Form/Module.bsl",
        "DataProcessors/Orphan/Ext/ManagerModule.bsl",
        "DataProcessors/Orphan/Forms/Main/Ext/Form.xml",
        "DataProcessors/Processor/Forms/Ghost/Ext/Form.xml",
        "DataProcessors/Processor/Forms/Ghost/Ext/Form/Module.bsl",
        "Documents/Invoice/Ext/RecordSetModule.bsl",
        "Documents/Invoice/Ext/ManagerModule.bsl",
        "Ext/ExternalConnectionModule.bsl",
        "DataProcessors/Absent/Ext/ManagerModule.bsl",
    ]
    .iter()
    .enumerate()
    {
        let result = mcp.completed_tool_call(call_tool(
            index as u64 + 2,
            "unica.resolve",
            json!({"path":path}),
        ));
        assert_eq!(result["ok"], false, "{path}: {result:#}");
        assert_eq!(
            result["diagnostics"][0]["code"], "not_found",
            "{path}: {result:#}"
        );
    }
    mcp.finish();
}

#[test]
fn resolve_form_metadata_survives_absent_local_content_without_substituting_it() {
    for source_kind in ["CONFIGURATION", "EXTENSION"] {
        let root = resource_resolve_fixture();
        let manifest = root.path().join("v8project.yaml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap()
            .replace("CONFIGURATION", source_kind);
        std::fs::write(manifest, text).unwrap();
        for prefix in ["CommonForms/Main", "DataProcessors/Processor/Forms/Main"] {
            std::fs::remove_file(root.path().join("src").join(prefix).join("Ext/Form.xml"))
                .unwrap();
        }
        let mut mcp = resource_resolve_mcp(root.path());
        let mut id = 2;
        for (prefix, address) in [
            ("CommonForms/Main", "main:CommonForm.Main"),
            (
                "DataProcessors/Processor/Forms/Main",
                "main:DataProcessor.Processor.Form.Main",
            ),
        ] {
            for args in [
                json!({"at":format!("{address}.Metadata")}),
                json!({"path":format!("src/{prefix}.xml")}),
            ] {
                let metadata = mcp.completed_tool_call(call_tool(id, "unica.resolve", args));
                id += 1;
                assert_eq!(metadata["ok"], true, "{source_kind}: {metadata:#}");
                assert_eq!(metadata["data"]["at"], format!("{address}.Metadata"));
                assert_eq!(metadata["data"]["path"], format!("{prefix}.xml"));
                assert_eq!(metadata["data"]["kind"], "Metadata");
            }
            let missing =
                mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"at":address})));
            id += 1;
            assert_eq!(
                missing["ok"], false,
                "missing content must not resolve to metadata: {missing:#}"
            );
            // These descriptors are own objects, including in the extension:
            // a missing companion is a source error, not inherited content.
            assert_eq!(missing["diagnostics"][0]["code"], "provider_unavailable");
            assert_eq!(missing["diagnostics"][0]["detailCode"], "source_unreadable");
        }
        mcp.finish();
    }
}

#[test]
fn resolve_resources_keep_source_prefix_and_deepest_root_selection_on_public_mcp() {
    let root = resource_resolve_fixture();
    copy_tree(&root.path().join("src"), root.path());
    std::fs::write(root.path().join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: outer\n    type: CONFIGURATION\n    path: .\n  - name: inner\n    type: CONFIGURATION\n    path: src\n").unwrap();
    let mut mcp = resource_resolve_mcp(root.path());
    let path = "DataProcessors/Processor/Forms/Main/Ext/Form/Module.bsl";
    for (index, query) in [
        format!("src/{path}"),
        std::fs::canonicalize(root.path().join("src").join(path))
            .unwrap()
            .to_str()
            .unwrap()
            .to_string(),
    ]
    .iter()
    .enumerate()
    {
        let result = mcp.completed_tool_call(call_tool(
            index as u64 + 2,
            "unica.resolve",
            json!({"path":query}),
        ));
        assert_eq!(result["ok"], true, "{result:#}");
        assert_eq!(
            result["data"]["at"],
            "inner:DataProcessor.Processor.Form.Main.Module.Form"
        );
        assert_eq!(result["data"]["path"], path);
    }
    let ambiguous = mcp.completed_tool_call(call_tool(5, "unica.resolve", json!({"path":path})));
    assert_eq!(ambiguous["ok"], false, "{ambiguous:#}");
    assert_eq!(ambiguous["diagnostics"][0]["code"], "bad_value");
    // Незарегистрированный физический двойник должен приводить к отказу
    // из-за неоднозначности, а не к выбору произвольного источника до проверки
    // типизированной регистрации.
    let root_descriptor = root.path().join("Configuration.xml");
    let text = std::fs::read_to_string(&root_descriptor)
        .unwrap()
        .replace("<DataProcessor>Processor</DataProcessor>", "");
    std::fs::write(&root_descriptor, text).unwrap();
    let ambiguous = mcp.completed_tool_call(call_tool(6, "unica.resolve", json!({"path":path})));
    assert_eq!(ambiguous["ok"], false, "{ambiguous:#}");
    assert_eq!(ambiguous["diagnostics"][0]["code"], "bad_value");
    let module_path = "Documents/Invoice/Ext/ObjectModule.bsl";
    let ambiguous =
        mcp.completed_tool_call(call_tool(7, "unica.resolve", json!({"path":module_path})));
    assert_eq!(ambiguous["ok"], false, "{ambiguous:#}");
    assert_eq!(ambiguous["diagnostics"][0]["code"], "bad_value");
    let text = std::fs::read_to_string(&root_descriptor)
        .unwrap()
        .replace("<Document>Invoice</Document>", "");
    std::fs::write(root_descriptor, text).unwrap();
    let ambiguous =
        mcp.completed_tool_call(call_tool(8, "unica.resolve", json!({"path":module_path})));
    assert_eq!(ambiguous["ok"], false, "{ambiguous:#}");
    assert_eq!(ambiguous["diagnostics"][0]["code"], "bad_value");
    mcp.finish();
}

#[cfg(unix)]
#[test]
fn resolve_resources_do_not_follow_linked_files_or_directories_on_public_mcp() {
    use std::os::unix::fs::symlink;
    for relative in [
        "DataProcessors/Processor/Forms/Main/Ext/Form/Module.bsl",
        "DataProcessors/Processor/Forms/Main/Ext",
        "DataProcessors/Processor/Forms/Main",
        "DataProcessors/Processor/Forms/Main.xml",
    ] {
        let root = resource_resolve_fixture();
        let target = root.path().join("src").join(relative);
        let foreign = root.path().join("foreign");
        std::fs::rename(&target, &foreign).unwrap();
        symlink(&foreign, &target).unwrap();
        let mut mcp = resource_resolve_mcp(root.path());
        let result = mcp.completed_tool_call(call_tool(
            2,
            "unica.resolve",
            json!({"path":"src/DataProcessors/Processor/Forms/Main/Ext/Form/Module.bsl"}),
        ));
        assert_eq!(result["ok"], false, "{relative}: {result:#}");
        assert!(
            ["invalid_source", "provider_unavailable"]
                .contains(&result["diagnostics"][0]["code"].as_str().unwrap()),
            "{relative}: {result:#}"
        );
        mcp.finish();
    }
}

#[cfg(unix)]
#[test]
fn resolve_resource_prefix_skips_a_foreign_linked_collection_on_public_mcp() {
    use std::os::unix::fs::symlink;
    let root = resource_resolve_fixture();
    let other = root.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::copy(
        root.path().join("src/Configuration.xml"),
        other.join("Configuration.xml"),
    )
    .unwrap();
    symlink(
        root.path().join("src/DataProcessors"),
        other.join("DataProcessors"),
    )
    .unwrap();
    std::fs::write(root.path().join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: foreign\n    type: CONFIGURATION\n    path: other\n").unwrap();
    let mut mcp = resource_resolve_mcp(root.path());
    let result = mcp.completed_tool_call(call_tool(
        2,
        "unica.resolve",
        json!({"path":"src/DataProcessors/Processor/Forms/Main/Ext/Form/Module.bsl"}),
    ));
    assert_eq!(result["ok"], true, "{result:#}");
    assert_eq!(
        result["data"]["at"],
        "main:DataProcessor.Processor.Form.Main.Module.Form"
    );
    mcp.finish();
}

#[test]
fn resolve_resource_refuses_mismatched_descriptor_identity_on_public_mcp() {
    for (relative, name) in [
        ("DataProcessors/Processor.xml", "Processor"),
        ("DataProcessors/Processor/Forms/Main.xml", "Main"),
        ("CommonForms/Main.xml", "Main"),
    ] {
        let root = resource_resolve_fixture();
        let path = root.path().join("src").join(relative);
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace(&format!("<Name>{name}</Name>"), "<Name>Other</Name>");
        std::fs::write(path, text).unwrap();
        let mut mcp = resource_resolve_mcp(root.path());
        let query = if relative.starts_with("CommonForms") {
            "src/CommonForms/Main/Ext/Form.xml"
        } else {
            "src/DataProcessors/Processor/Forms/Main/Ext/Form.xml"
        };
        let result = mcp.completed_tool_call(call_tool(2, "unica.resolve", json!({"path":query})));
        assert_eq!(result["ok"], false, "{relative}: {result:#}");
        assert_eq!(
            result["diagnostics"][0]["code"], "invalid_source",
            "{result:#}"
        );
        mcp.finish();
    }
}

#[test]
fn resolve_common_form_resources_preserve_deepest_root_and_ambiguity() {
    let root = resource_resolve_fixture();
    copy_tree(&root.path().join("src"), root.path());
    std::fs::write(root.path().join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: outer\n    type: CONFIGURATION\n    path: .\n  - name: inner\n    type: CONFIGURATION\n    path: src\n").unwrap();
    let mut mcp = resource_resolve_mcp(root.path());
    let mut id = 2;
    for (path, suffix) in [
        ("CommonForms/Main.xml", ".Metadata"),
        ("CommonForms/Main/Ext/Form.xml", ""),
        ("CommonForms/Main/Ext/Form/Module.bsl", ".Module.Form"),
    ] {
        for query in [
            format!("src/{path}"),
            std::fs::canonicalize(root.path().join("src").join(path))
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
        ] {
            let result =
                mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"path":query})));
            id += 1;
            assert_eq!(result["ok"], true, "{result:#}");
            assert_eq!(
                result["data"]["at"],
                format!("inner:CommonForm.Main{suffix}")
            );
            assert_eq!(result["data"]["path"], path);
        }
        let ambiguous =
            mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"path":path})));
        id += 1;
        assert_eq!(ambiguous["ok"], false, "{ambiguous:#}");
        assert_eq!(ambiguous["diagnostics"][0]["code"], "bad_value");
    }
    mcp.finish();
}

#[cfg(unix)]
#[test]
fn resolve_common_form_resources_never_follow_content_or_metadata_links() {
    use std::os::unix::fs::symlink;
    for relative in [
        "CommonForms/Main.xml",
        "CommonForms/Main/Ext/Form.xml",
        "CommonForms/Main/Ext/Form/Module.bsl",
        "CommonForms/Main/Ext",
    ] {
        let root = resource_resolve_fixture();
        let target = root.path().join("src").join(relative);
        let foreign = root.path().join("foreign");
        std::fs::rename(&target, &foreign).unwrap();
        symlink(&foreign, &target).unwrap();
        let mut mcp = resource_resolve_mcp(root.path());
        for (id, query) in (2..).zip([
            "CommonForms/Main.xml",
            "CommonForms/Main/Ext/Form.xml",
            "CommonForms/Main/Ext/Form/Module.bsl",
        ]) {
            let result =
                mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"path":query})));
            if relative == "CommonForms/Main.xml"
                || query.starts_with(relative)
                || relative.ends_with("/Ext") && query.contains("/Ext/")
            {
                assert_eq!(result["ok"], false, "{relative} {query}: {result:#}");
                assert!(
                    ["invalid_source", "provider_unavailable"]
                        .contains(&result["diagnostics"][0]["code"].as_str().unwrap()),
                    "{result:#}"
                );
            }
        }
        mcp.finish();
    }
}

fn external_form_resolve_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("external form resolve workspace");
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-external"),
        root.path(),
    );
    root
}

#[test]
fn resolve_external_form_resources_round_trip_all_three_file_kinds() {
    let root = external_form_resolve_fixture();
    let mut mcp = resource_resolve_mcp(root.path());
    let mut id = 2;
    for (set, owner, kind) in [
        ("epf", "Import", "ExternalDataProcessor"),
        ("erf", "Sales", "ExternalReport"),
    ] {
        let form = format!("{set}:{kind}.{owner}.Form.Main");
        for (path, suffix, expected_kind) in [
            (format!("{owner}/Forms/Main.xml"), ".Metadata", "Metadata"),
            (format!("{owner}/Forms/Main/Ext/Form.xml"), "", "Form"),
            (
                format!("{owner}/Forms/Main/Ext/Form/Module.bsl"),
                ".Module.Form",
                "Module",
            ),
        ] {
            let address = format!("{form}{suffix}");
            for query in [
                path.clone(),
                format!("{set}/{path}"),
                std::fs::canonicalize(root.path().join(set).join(&path))
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
            ] {
                let resolved =
                    mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"path":query})));
                id += 1;
                assert_eq!(resolved["ok"], true, "{query}: {resolved:#}");
                assert_eq!(resolved["data"]["at"], address);
                assert_eq!(resolved["data"]["path"], path);
                assert_eq!(resolved["data"]["kind"], expected_kind);
                let reverse =
                    mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"at":address})));
                id += 1;
                assert_eq!(reverse["ok"], true, "{address}: {reverse:#}");
                assert_eq!(reverse["data"]["at"], address);
                assert_eq!(reverse["data"]["path"], path);
                assert_eq!(reverse["data"]["kind"], expected_kind);
            }
        }
    }
    mcp.finish();
}

#[test]
fn resolve_external_form_resources_require_registered_matching_owner_and_form() {
    for (set, owner) in [("epf", "Import"), ("erf", "Sales")] {
        for fault in [
            "unregistered-form",
            "wrong-form-name",
            "wrong-owner-name",
            "absent-content",
        ] {
            let root = external_form_resolve_fixture();
            let source = root.path().join(set);
            let owner_descriptor = source.join(format!("{owner}.xml"));
            let form_descriptor = source.join(format!("{owner}/Forms/Main.xml"));
            match fault {
                "unregistered-form" => {
                    let text = std::fs::read_to_string(&owner_descriptor)
                        .unwrap()
                        .replace("<Form>Main</Form>", "");
                    std::fs::write(&owner_descriptor, text).unwrap();
                }
                "wrong-form-name" => {
                    let text = std::fs::read_to_string(&form_descriptor)
                        .unwrap()
                        .replace("<Name>Main</Name>", "<Name>Other</Name>");
                    std::fs::write(&form_descriptor, text).unwrap();
                }
                "wrong-owner-name" => {
                    let text = std::fs::read_to_string(&owner_descriptor)
                        .unwrap()
                        .replace(&format!("<Name>{owner}</Name>"), "<Name>Other</Name>");
                    std::fs::write(&owner_descriptor, text).unwrap();
                }
                _ => std::fs::remove_file(source.join(format!("{owner}/Forms/Main/Ext/Form.xml")))
                    .unwrap(),
            }
            let mut mcp = resource_resolve_mcp(root.path());
            let paths = if fault == "absent-content" {
                vec![format!("{set}/{owner}/Forms/Main/Ext/Form.xml")]
            } else {
                vec![
                    format!("{set}/{owner}/Forms/Main.xml"),
                    format!("{set}/{owner}/Forms/Main/Ext/Form.xml"),
                    format!("{set}/{owner}/Forms/Main/Ext/Form/Module.bsl"),
                ]
            };
            for (index, path) in paths.into_iter().enumerate() {
                let resolved = mcp.completed_tool_call(call_tool(
                    index as u64 + 2,
                    "unica.resolve",
                    json!({"path":path}),
                ));
                assert_eq!(resolved["ok"], false, "{set}/{fault}: {resolved:#}");
                assert!(
                    ["not_found", "invalid_source"]
                        .contains(&resolved["diagnostics"][0]["code"].as_str().unwrap()),
                    "{resolved:#}"
                );
            }
            mcp.finish();
        }
    }
}

#[cfg(unix)]
#[test]
fn resolve_external_form_resources_never_follow_linked_content() {
    use std::os::unix::fs::symlink;
    for (set, owner) in [("epf", "Import"), ("erf", "Sales")] {
        let root = external_form_resolve_fixture();
        let source = root.path().join(set);
        let content = source.join(format!("{owner}/Forms/Main/Ext"));
        let foreign = root.path().join("foreign");
        std::fs::rename(&content, &foreign).unwrap();
        symlink(&foreign, &content).unwrap();
        let mut mcp = resource_resolve_mcp(root.path());
        for (index, tail) in ["Form.xml", "Form/Module.bsl"].iter().enumerate() {
            let resolved = mcp.completed_tool_call(call_tool(
                index as u64 + 2,
                "unica.resolve",
                json!({"path":format!("{set}/{owner}/Forms/Main/Ext/{tail}")}),
            ));
            assert_eq!(resolved["ok"], false, "{set}: {resolved:#}");
            assert!(
                ["invalid_source", "provider_unavailable"]
                    .contains(&resolved["diagnostics"][0]["code"].as_str().unwrap()),
                "{resolved:#}"
            );
        }
        mcp.finish();
    }
}
