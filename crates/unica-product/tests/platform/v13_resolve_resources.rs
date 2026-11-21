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

fn resolve_form_and_module_paths_preserve_the_exact_resource(source_kind: &str) {
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
                let result =
                    mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"path":query})));
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
                let reverse =
                    mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"at": address})));
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
        let result = mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"path":path})));
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
#[test]
fn resolve_form_and_module_paths_preserve_the_exact_resource_of_a_configuration_on_public_mcp() {
    resolve_form_and_module_paths_preserve_the_exact_resource("CONFIGURATION");
}

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
#[test]
fn resolve_form_and_module_paths_preserve_the_exact_resource_of_an_extension_on_public_mcp() {
    resolve_form_and_module_paths_preserve_the_exact_resource("EXTENSION");
}

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
/// rule: INV.SOURCE.SOURCE-PATH-RESOLUTION
#[test]
fn resolve_bot_module_round_trips_its_native_platform_path() {
    let root = tempfile::tempdir().unwrap();
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/formats/platform_xml/2.20/all_kinds"),
        root.path(),
    );
    let source = root.path().join("src");
    let path = "Bots/AllKindsBot/Ext/Module.bsl";
    let address = "main:Bot.AllKindsBot.Module.Bot";
    let wrong_role = "Bots/AllKindsBot/Ext/ObjectModule.bsl";
    let unregistered = "Bots/OrphanBot/Ext/Module.bsl";
    std::fs::create_dir_all(source.join("Bots/OrphanBot/Ext")).unwrap();
    let descriptor = std::fs::read_to_string(source.join("Bots/AllKindsBot.xml")).unwrap();
    std::fs::write(
        source.join("Bots/OrphanBot.xml"),
        descriptor.replace("<Name>AllKindsBot</Name>", "<Name>OrphanBot</Name>"),
    )
    .unwrap();
    for misplaced in [wrong_role, unregistered] {
        std::fs::copy(source.join(path), source.join(misplaced)).unwrap();
    }

    let mut mcp = resource_resolve_mcp(root.path());
    for (id, args) in [
        (2, json!({"at":address})),
        (3, json!({"path":path})),
        (4, json!({"path":format!("src/{path}")})),
    ] {
        let result = mcp.completed_tool_call(call_tool(id, "unica.resolve", args));
        assert_eq!(result["ok"], true, "{result:#}");
        assert_eq!(result["data"]["at"], address);
        assert_eq!(result["data"]["path"], path);
        assert_eq!(result["data"]["kind"], "Module");
    }
    for (id, path) in [(5, wrong_role), (6, unregistered)] {
        let result = mcp.completed_tool_call(call_tool(id, "unica.resolve", json!({"path":path})));
        assert_eq!(result["ok"], false, "{path}: {result:#}");
        assert_eq!(result["diagnostics"][0]["code"], "not_found", "{result:#}");
    }
    mcp.finish();
}

const SEQUENCE_RECORD_SET: &str = "main:Sequence.Probe.Module.RecordSet";
const SEQUENCE_RECORD_SET_PATH: &str = "Sequences/Probe/Ext/RecordSetModule.bsl";
const SEQUENCE_PROBE_METHOD: &str = "UnicaSequenceRoundTripProbe";
const SEQUENCE_FIXTURES: [(&str, &str); 3] = [
    (
        "2.20",
        "../../tests/fixtures/formats/platform_xml/2.20/all_kinds",
    ),
    (
        "2.21",
        "../../tests/fixtures/formats/platform_xml/2.21/all_kinds",
    ),
    (
        "2.22",
        "../../tests/fixtures/formats/platform_xml/2.22/all_kinds",
    ),
];

fn sequence_module_workspace(fixture: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture),
        root.path(),
    );
    root
}

/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
#[test]
fn sequence_record_set_module_is_reachable_and_resolves_its_native_path() {
    for (format, fixture) in SEQUENCE_FIXTURES {
        let root = sequence_module_workspace(fixture);
        let src = root.path().join("src");
        let native = src.join(SEQUENCE_RECORD_SET_PATH);
        let wrong_role = "Sequences/Probe/Ext/ObjectModule.bsl";
        let wrong_kind = "Documents/Orders/Ext/RecordSetModule.bsl";
        let unregistered = "Sequences/Orphan/Ext/RecordSetModule.bsl";
        for relative in [wrong_role, wrong_kind, unregistered] {
            let path = src.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::copy(&native, path).unwrap();
        }
        let descriptor = std::fs::read_to_string(src.join("Sequences/Probe.xml")).unwrap();
        std::fs::write(
            src.join("Sequences/Orphan.xml"),
            descriptor.replace("Probe", "Orphan"),
        )
        .unwrap();

        let mut mcp = resource_resolve_mcp(root.path());
        let mut id = 1_u64;
        let mut call = |tool, args| {
            id += 1;
            mcp.completed_tool_call(call_tool(id, tool, args))
        };
        let branch_at = "main:Sequence.Probe.Module";
        let owner = call("unica.view", json!({"at":"main:Sequence.Probe"}));
        assert_eq!(owner["ok"], true, "{format}: {owner:#}");
        assert!(owner["data"]["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|branch| branch["at"] == branch_at && branch["count"] == 1));
        let branch = call("unica.view", json!({"at":branch_at}));
        assert_eq!(branch["ok"], true, "{format}: {branch:#}");
        assert_eq!(branch["data"]["items"][0]["at"], SEQUENCE_RECORD_SET);
        let method_at = format!("{SEQUENCE_RECORD_SET}.Method.{SEQUENCE_PROBE_METHOD}");
        let method = call("unica.view", json!({"at":method_at}));
        assert_eq!(method["ok"], true, "{format}: {method:#}");
        assert_eq!(method["data"]["kind"], "Method");

        for query in [
            json!({"at":SEQUENCE_RECORD_SET}),
            json!({"path":SEQUENCE_RECORD_SET_PATH}),
            json!({"path":format!("src/{SEQUENCE_RECORD_SET_PATH}")}),
        ] {
            let result = call("unica.resolve", query);
            assert_eq!(result["ok"], true, "{format}: {result:#}");
            assert_eq!(result["data"]["at"], SEQUENCE_RECORD_SET);
            assert_eq!(result["data"]["path"], SEQUENCE_RECORD_SET_PATH);
            assert_eq!(result["data"]["kind"], "Module");
        }
        for relative in [wrong_role, wrong_kind, unregistered] {
            let result = call("unica.resolve", json!({"path":relative}));
            assert_eq!(result["ok"], false, "{format}: {relative}: {result:#}");
            assert_eq!(result["diagnostics"][0]["code"], "not_found");
        }
        mcp.finish();
    }
}

/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
/// rule: INV.SOURCE.TAIL-INSERT
#[test]
fn sequence_record_set_first_insert_and_blank_replacement_use_public_apply() {
    for (format, fixture) in SEQUENCE_FIXTURES {
        let root = sequence_module_workspace(fixture);
        let src = root.path().join("src");
        let path = src.join(SEQUENCE_RECORD_SET_PATH);
        let native = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(src.join("Sequences/Probe/Ext")).unwrap();
        std::fs::remove_dir(src.join("Sequences/Probe")).unwrap();
        let owner = src.join("Sequences/Probe.xml");
        let neighbor = src.join("Documents/Orders.xml");
        let owner_before = std::fs::read(&owner).unwrap();
        let neighbor_before = std::fs::read(&neighbor).unwrap();

        let mut mcp = resource_resolve_mcp(root.path());
        let mut id = 1_u64;
        let mut call = |tool, args| {
            id += 1;
            mcp.completed_tool_call(call_tool(id, tool, args))
        };
        let missing = call("unica.resolve", json!({"path":SEQUENCE_RECORD_SET_PATH}));
        assert_eq!(missing["ok"], false, "{format}: {missing:#}");
        assert_eq!(missing["diagnostics"][0]["code"], "not_found");
        let module = call("unica.view", json!({"at":SEQUENCE_RECORD_SET}));
        assert_eq!(module["ok"], true, "{format}: {module:#}");
        assert_eq!(module["data"]["props"]["role"], "RecordSet");

        let body = format!("{SEQUENCE_RECORD_SET}.Body");
        for (op, at, text) in [
            (
                "code.insert",
                SEQUENCE_RECORD_SET,
                native.trim_start_matches('\u{feff}'),
            ),
            ("code.replace", body.as_str(), "\n"),
        ] {
            let before = path.is_file().then(|| std::fs::read(&path).unwrap());
            let plan = call(
                "unica.apply",
                json!({"at":at,"ops":[{"op":op,"args":{"text":text}}]}),
            );
            assert_eq!(plan["ok"], true, "{format} {op}: {plan:#}");
            assert_eq!(
                path.is_file().then(|| std::fs::read(&path).unwrap()),
                before
            );
            let token = plan["data"]["executionToken"]
                .as_str()
                .expect("execution token");
            let applied = call("unica.apply", json!({"executionToken":token}));
            assert_eq!(applied["ok"], true, "{format} {op}: {applied:#}");
            let after = std::fs::read_to_string(&path).unwrap();
            if op == "code.insert" {
                assert!(after.contains(SEQUENCE_PROBE_METHOD), "{format}: {after:?}");
            } else {
                assert!(
                    matches!(after.trim_start_matches('\u{feff}'), "\n" | "\r\n"),
                    "{format}: {after:?}"
                );
            }
            assert_eq!(std::fs::read(&owner).unwrap(), owner_before);
            assert_eq!(std::fs::read(&neighbor).unwrap(), neighbor_before);
        }
        mcp.finish();
    }
}

const RECALCULATION: &str = "main:CalculationRegister.Начисления.Recalculation.Перерасчет1";
const RECALCULATION_DESCRIPTOR: &str =
    "CalculationRegisters/Начисления/Recalculations/Перерасчет1.xml";
const RECALCULATION_MODULE: &str =
    "CalculationRegisters/Начисления/Recalculations/Перерасчет1/Ext/RecordSetModule.bsl";
const RECALCULATION_METHOD: &str = "UnicaRecalculationRoundTripProbe";

fn recalculation_workspace() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/formats/platform_xml/2.20/recalculation"),
        root.path(),
    );
    root
}

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
#[test]
fn recalculation_record_set_body_and_resolve_require_registered_owners() {
    let root = recalculation_workspace();
    let src = root.path().join("src");
    let owner_branch = "main:CalculationRegister.Начисления.Recalculation";
    let module = format!("{RECALCULATION}.Module.RecordSet");
    let body = format!("{module}.Body");
    let wrong_role = RECALCULATION_MODULE.replace("RecordSetModule", "ManagerModule");
    let wrong_parent =
        RECALCULATION_MODULE.replace("CalculationRegisters/Начисления", "Documents/Приход");
    let orphan = RECALCULATION_MODULE.replace("Перерасчет1", "Orphan");
    let orphan_descriptor = RECALCULATION_DESCRIPTOR.replace("Перерасчет1", "Orphan");
    for path in [&wrong_role, &wrong_parent, &orphan] {
        let path = src.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::copy(src.join(RECALCULATION_MODULE), path).unwrap();
    }
    let descriptor = std::fs::read_to_string(src.join(RECALCULATION_DESCRIPTOR)).unwrap();
    std::fs::write(
        src.join(orphan_descriptor),
        descriptor.replace("Перерасчет1", "Orphan"),
    )
    .unwrap();

    let mut mcp = resource_resolve_mcp(root.path());
    let mut id = 1_u64;
    let mut call = |tool, args| {
        id += 1;
        mcp.completed_tool_call(call_tool(id, tool, args))
    };
    let branch = call("unica.view", json!({"at":owner_branch}));
    assert_eq!(branch["ok"], true, "{branch:#}");
    assert_eq!(branch["data"]["items"][0]["at"], RECALCULATION);
    let owner = call("unica.view", json!({"at":RECALCULATION}));
    assert_eq!(owner["ok"], true, "{owner:#}");
    let module_branch = format!("{RECALCULATION}.Module");
    assert!(owner["data"]["branches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|branch| branch["at"] == module_branch && branch["count"] == 1));
    let roles = call("unica.view", json!({"at":module_branch}));
    assert_eq!(roles["ok"], true, "{roles:#}");
    assert_eq!(roles["data"]["items"][0]["at"], module);

    for query in [
        json!({"at":module}),
        json!({"path":RECALCULATION_MODULE}),
        json!({"path":format!("src/{RECALCULATION_MODULE}")}),
    ] {
        let resolved = call("unica.resolve", query);
        assert_eq!(resolved["ok"], true, "{resolved:#}");
        assert_eq!(resolved["data"]["at"], module);
        assert_eq!(resolved["data"]["path"], RECALCULATION_MODULE);
        assert_eq!(resolved["data"]["kind"], "Module");
    }
    for path in [wrong_role, wrong_parent, orphan] {
        let result = call("unica.resolve", json!({"path":path}));
        assert_eq!(result["ok"], false, "{result:#}");
        assert_eq!(result["diagnostics"][0]["code"], "not_found");
    }
    let orphan_at = format!("{owner_branch}.Orphan.Module.RecordSet");
    for (tool, args) in [
        ("unica.view", json!({"at":orphan_at})),
        ("unica.resolve", json!({"at":orphan_at})),
        (
            "unica.apply",
            json!({"at":orphan_at,"ops":[{"op":"code.insert","args":{"text":"// edit"}}]}),
        ),
    ] {
        let result = call(tool, args);
        assert_eq!(result["ok"], false, "{tool}: {result:#}");
        assert_eq!(result["diagnostics"][0]["code"], "not_found");
    }

    let response = mcp.exchange(call_tool(id + 1, "unica.view", json!({"at":body})));
    let content = response["result"]["content"].as_array().unwrap();
    let envelope: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(envelope["ok"], true, "{envelope:#}");
    assert_eq!(envelope["data"]["at"], body);
    assert_eq!(envelope["data"]["kind"], "Body");
    assert!(content[1]["text"]
        .as_str()
        .unwrap()
        .contains(RECALCULATION_METHOD));
    mcp.finish();
}

/// rule: INV.SOURCE.TAIL-INSERT
#[test]
fn recalculation_record_set_materializes_and_clears_through_public_apply() {
    let root = recalculation_workspace();
    let src = root.path().join("src");
    let path = src.join(RECALCULATION_MODULE);
    let native = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let ext = path.parent().unwrap();
    std::fs::remove_dir(ext).unwrap();
    std::fs::remove_dir(ext.parent().unwrap()).unwrap();
    let unchanged = [
        src.join(RECALCULATION_DESCRIPTOR),
        src.join("CalculationRegisters/Начисления.xml"),
        src.join("CalculationRegisters/Начисления/Ext/RecordSetModule.bsl"),
    ]
    .map(|path| {
        let before = std::fs::read(&path).unwrap();
        (path, before)
    });

    let mut mcp = resource_resolve_mcp(root.path());
    let mut id = 1_u64;
    let mut call = |tool, args| {
        id += 1;
        mcp.completed_tool_call(call_tool(id, tool, args))
    };
    let module = format!("{RECALCULATION}.Module.RecordSet");
    let missing = call("unica.resolve", json!({"path":RECALCULATION_MODULE}));
    assert_eq!(missing["ok"], false, "{missing:#}");
    assert_eq!(missing["diagnostics"][0]["code"], "not_found");
    let view = call("unica.view", json!({"at":module}));
    assert_eq!(view["ok"], true, "{view:#}");
    assert_eq!(view["data"]["props"]["role"], "RecordSet");

    let body = format!("{module}.Body");
    for (at, op, text) in [
        (
            module.as_str(),
            "code.insert",
            native.trim_start_matches('\u{feff}'),
        ),
        (body.as_str(), "code.replace", "\n"),
    ] {
        let before = std::fs::read(&path).ok();
        let plan = call(
            "unica.apply",
            json!({"at":at,"ops":[{"op":op,"args":{"text":text}}]}),
        );
        assert_eq!(plan["ok"], true, "{op}: {plan:#}");
        assert_eq!(std::fs::read(&path).ok(), before);
        let token = plan["data"]["executionToken"].as_str().unwrap();
        let applied = call("unica.apply", json!({"executionToken":token}));
        assert_eq!(applied["ok"], true, "{op}: {applied:#}");
        let after = std::fs::read_to_string(&path).unwrap();
        if op == "code.insert" {
            assert!(after.contains(RECALCULATION_METHOD), "{after:?}");
        } else {
            assert!(matches!(
                after.trim_start_matches('\u{feff}'),
                "\n" | "\r\n"
            ));
        }
        for (path, before) in &unchanged {
            assert_eq!(std::fs::read(path).unwrap(), *before, "{path:?}");
        }
    }
    mcp.finish();
}

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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
                if fault == "wrong-owner-name" {
                    // The descriptor's declared identity is independent of its
                    // export filename; children stay under the physical stem.
                    assert_eq!(resolved["ok"], true, "{set}/{fault}: {resolved:#}");
                    assert!(resolved["data"]["at"]
                        .as_str()
                        .unwrap()
                        .contains(".Other.Form.Main"));
                    continue;
                }
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

/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
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

#[test]
fn resolve_external_alias_batch_lines_keep_point_owner_proof() {
    let root = tempfile::tempdir().unwrap();
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-external-alias"),
        root.path(),
    );
    for (set, stem) in [("epf", "source"), ("erf", "report-source")] {
        std::fs::write(root.path().join(set).join("broken.xml"), "<broken").unwrap();
        for module in ["Ext/ObjectModule.bsl", "Forms/Main/Ext/Form/Module.bsl"] {
            std::fs::write(
                root.path().join(set).join(stem).join(module),
                "Procedure Probe() Export\nEndProcedure\n// outside\n",
            )
            .unwrap();
        }
    }
    let mut mcp = resource_resolve_mcp(root.path());
    let mut id = 2;
    for (set, stem, kind, name) in [
        (
            "epf",
            "source",
            "ExternalDataProcessor",
            "SyntheticProcessor",
        ),
        ("erf", "report-source", "ExternalReport", "SyntheticReport"),
    ] {
        for (module, suffix) in [
            ("Ext/ObjectModule.bsl", ".Module.Object"),
            ("Forms/Main/Ext/Form/Module.bsl", ".Form.Main.Module.Form"),
        ] {
            let path = format!("{set}/{stem}/{module}");
            let address = format!("{set}:{kind}.{name}{suffix}");
            let batch = mcp.completed_tool_call(call_tool(
                id,
                "unica.resolve",
                json!({"paths": [path, format!("{path}:1"), format!("{path}:3")]}),
            ));
            id += 1;
            assert_eq!(batch["ok"], true, "{batch:#}");
            let items = batch["data"]["items"].as_array().unwrap();
            assert_eq!(items[0]["at"], address, "{batch:#}");
            assert_eq!(
                items[1]["at"],
                format!("{address}.Method.Probe"),
                "{batch:#}"
            );
            assert_eq!(items[1]["line"], "inMethod");
            assert_eq!(items[2]["at"], address, "{batch:#}");
            assert_eq!(items[2]["line"], "outsideMethod");
        }
    }
    mcp.finish();
}
