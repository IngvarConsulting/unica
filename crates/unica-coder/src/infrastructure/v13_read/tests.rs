// Submodules are the files of `tests/`; build.rs declares them, so a new test
// file needs no registration here.
include!(concat!(env!("OUT_DIR"), "/v13_read_tests_modules.rs"));

use support::*;

use super::{
    project_known_suffix, project_typed_payload, resolve_platform_xml_target,
    LogicalViewReadAuthority, MetadataAddress, SourceTarget, TargetKindPolicy,
};
use crate::application::v13::find::FindRequest;
use crate::infrastructure::logical_tree::route_logical_address;
use unica_address::address::QualifiedAddress;
use unica_bsl::profile::PlatformProfile;
use unica_navigation::result_store::ViewCursorStore;
use unica_refusal::RefusalCode;
use {
    crate::application::v13::view::ViewReadAuthority, crate::application::v13::view::ViewRequest,
    crate::application::v13::view::ViewService, unica_execution_contract::view_error::ViewError,
};

use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::SourceSetKind;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::test_support::supports_retained_root_replacement_test;

use crate::infrastructure::v13_read_port::{
    review_clear_revision_identity_hooks, review_set_revision_identity_hooks, ProviderReadAuthority,
};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const COMMON_MODULE_PROPERTY_KEYS: [&str; 8] = [
    "global",
    "clientManagedApplication",
    "server",
    "externalConnection",
    "clientOrdinaryApplication",
    "serverCall",
    "privileged",
    "returnValuesReuse",
];

fn configuration_payload(
    reader: &ProviderReadAuthority,
) -> Result<serde_json::Value, unica_execution_contract::view_error::ViewError> {
    reader
        .configuration_payload_with_checkpoint(&mut || Ok(()))
        .map(|payload| payload.as_ref().clone())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AcceptedAddressProfile {
    valid_addresses: Vec<AcceptedAddressCase>,
}

#[derive(Debug, Deserialize)]
struct AcceptedAddressCase {
    case: String,
    input: String,
}

fn subsystem_descriptor(name: &str, content: &[&str], children: &[&str]) -> String {
    let content = content
        .iter()
        .map(|item| format!(r#"<xr:Item xsi:type="xr:MDObjectRef">{item}</xr:Item>"#))
        .collect::<String>();
    let children = children
        .iter()
        .map(|child| format!("<Subsystem>{child}</Subsystem>"))
        .collect::<String>();
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
  <Subsystem><Properties><Name>{name}</Name><Synonym/><IncludeInCommandInterface>true</IncludeInCommandInterface><UseOneCommand>false</UseOneCommand><Content>{content}</Content></Properties><ChildObjects>{children}</ChildObjects></Subsystem>
</MetaDataObject>"#
    )
}

fn install_supported_subsystem_descriptors(fixture: &RealReaderFixture) {
    for (relative, name, children, uuid) in [
        (
            "Subsystems/Sales.xml",
            "Sales",
            &["Orders"][..],
            "11111111-1111-4111-8111-111111111111",
        ),
        (
            "Subsystems/Sales/Subsystems/Orders.xml",
            "Orders",
            &[][..],
            "22222222-2222-4222-8222-222222222222",
        ),
    ] {
        write(
            &fixture.source.join(relative),
            &subsystem_descriptor(name, &[], children).replacen(
                "<Subsystem>",
                &format!("<Subsystem uuid=\"{uuid}\">"),
                1,
            ),
        );
    }
}

fn view_items(
    result: &unica_execution_contract::invocation::DomainResult,
) -> Vec<serde_json::Value> {
    assert!(result.ok, "{:?}", refusal_codes(result));
    result.data.as_ref().unwrap()["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn view_branch_count(
    result: &unica_execution_contract::invocation::DomainResult,
    at: &str,
) -> Option<u64> {
    assert!(result.ok, "{:?}", refusal_codes(result));
    result.data.as_ref().unwrap()["branches"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|branch| branch["at"] == at)
        .and_then(|branch| branch["count"].as_u64())
}

fn large_external_inventory_fixture() -> RealExternalReaderFixture {
    let fixture = RealExternalReaderFixture::new();
    add_large_external_descriptors(&fixture, 5);
    fixture
}

fn add_large_external_descriptors(fixture: &RealExternalReaderFixture, count: usize) {
    for index in 0..count {
        let name = format!("Large{index}");
        write_external_artifact(&fixture.processor, "ExternalDataProcessor", &name);
        let path = fixture.processor.join(format!("{name}.xml"));
        let descriptor = fs::read_to_string(&path).unwrap();
        let padding = format!("<!--{}-->", "x".repeat(7 * 1024 * 1024));
        fs::write(
            &path,
            descriptor.replace("</MetaDataObject>", &format!("{padding}</MetaDataObject>")),
        )
        .unwrap();
    }
}

fn external_inventory_reader(fixture: &RealExternalReaderFixture) -> ProviderReadAuthority {
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.processor).unwrap());
    ProviderReadAuthority::new(
        "artifact_processor",
        "actor-external-complete",
        SourceSetKind::ExternalProcessor,
        root,
    )
}

fn assert_complete_large_external_inventory(payload: &serde_json::Value) {
    let expected = [
        "Import",
        "Large0",
        "Large1",
        "Large2",
        "Large3",
        "Large4",
        "Импорт",
    ]
    .map(|name| json!({"kind": "ExternalDataProcessor", "name": name}));
    assert_eq!(payload["registeredObjects"], json!(expected));
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModuleCapabilityFixture {
    module_capabilities: Vec<ModuleCapabilityCase>,
}

#[derive(Deserialize)]
struct ModuleCapabilityCase {
    at: String,
    exists: bool,
}

mod canonical_module_diagnostics {
    use super::*;
    use crate::infrastructure::diagnostics::CanonicalDiagnosticMapping;
    use std::time::Duration;
    use unica_address::source_target::TargetKind;
    use unica_diagnostics::coordinator::DiagnosticCoordinator;
    use unica_diagnostics::coordinator::DiagnosticMapping;
    use unica_diagnostics::model::*;
    use unica_source_contract::location::SourceLocation;

    fn request(mapping: &CanonicalDiagnosticMapping) -> DiagnosticRequest {
        DiagnosticRequest {
            action: DiagnosticAction::Analyze,
            source_set: mapping.target().source_set.clone(),
            metadata_path: mapping.target().metadata_path.clone(),
            filter: DiagnosticFilter::default(),
            range: None,
            limit: 200,
            timeout: Some(Duration::from_secs(5)),
        }
    }

    fn finding(path: &Path, code: &str) -> DiagnosticObservation {
        DiagnosticObservation::Diagnostic {
            provider: BSL_ANALYZER_PROVIDER,
            location: DiagnosticObservationLocation::Resource {
                handle: path.to_string_lossy().into_owned(),
            },
            focus: DiagnosticObservationFocus::Target,
            code: code.into(),
            severity: DiagnosticSeverity::Error,
            message: "fixture finding".into(),
            tags: Vec::new(),
        }
    }

    fn assert_mapping(
        authority: &LogicalViewReadAuthority<'_>,
        workspace: &WorkspaceContext,
        at: &str,
        expected: &str,
        physical: &Path,
    ) {
        let mapping = authority
            .diagnostic_mapping(&QualifiedAddress::parse(at).unwrap(), workspace)
            .unwrap_or_else(|error| panic!("{at}: {error:?}"));
        let target = MetadataAddress::parse_logical(expected).unwrap();
        assert_eq!(mapping.target().metadata_path.as_ref(), Some(&target));
        assert_eq!(mapping.target().target_kind, TargetKind::Module);
        let cancellation = CancellationToken::new();
        let context = mapping
            .resolve_context(&request(&mapping), workspace, &cancellation)
            .unwrap();
        let item = mapping
            .map_observation(finding(physical, "Requested"), &context, &cancellation)
            .unwrap();
        assert!(
            matches!(item, DiagnosticItem::Diagnostic {
            location: SourceLocation::Addressed { metadata_path: Some(path), target_kind: TargetKind::Module, .. }, ..
        } if path == target),
            "{at}: provider resource must map to proved module"
        );
    }

    #[test]
    fn external_object_form_and_body_use_proven_canonical_modules() {
        let fixture = RealExternalReaderFixture::new();
        for (set, kind, class, name, root) in [
            (
                "artifact_processor",
                SourceSetKind::ExternalProcessor,
                "ExternalDataProcessor",
                "Import",
                &fixture.processor,
            ),
            (
                "artifact_report",
                SourceSetKind::ExternalReport,
                "ExternalReport",
                "Sales",
                &fixture.report,
            ),
        ] {
            let form = root.join(format!("{name}/Forms/Main/Ext/Form/Module.bsl"));
            write(&form, "Procedure OnOpen()\nEndProcedure\n");
            let authority = fixture.operation_read_authority(set, kind);
            for tail in ["", ".Body"] {
                assert_mapping(
                    &authority,
                    &fixture.context,
                    &format!("{set}:{class}.{name}.Module.Object{tail}"),
                    &format!("{class}.{name}.ObjectModule"),
                    &root.join(format!("{name}/Ext/ObjectModule.bsl")),
                );
                assert_mapping(
                    &authority,
                    &fixture.context,
                    &format!("{set}:{class}.{name}.Form.Main.Module.Form{tail}"),
                    &format!("{class}.{name}.Form.Main.FormModule"),
                    &form,
                );
            }
        }
    }

    #[test]
    fn configuration_common_object_and_form_modules_keep_exact_diagnostic_targets() {
        let fixture = RealReaderFixture::new();
        write(
            &fixture.source.join("Catalogs/Items/Ext/ObjectModule.bsl"),
            "Procedure Run()\nEndProcedure\n",
        );
        write(
            &fixture.source.join("Catalogs/Items/Ext/ManagerModule.bsl"),
            "Procedure Manage()\nEndProcedure\n",
        );
        for authority in [
            fixture.operation_read_authority(),
            fixture.extension_read_authority(),
        ] {
            for (at, expected, relative) in [
                (
                    "CommonModule.РеактивныйСервер",
                    "CommonModule.РеактивныйСервер.Module",
                    "CommonModules/РеактивныйСервер/Ext/Module.bsl",
                ),
                (
                    "Catalog.Items.Module.Object",
                    "Catalog.Items.ObjectModule",
                    "Catalogs/Items/Ext/ObjectModule.bsl",
                ),
                (
                    "Catalog.Items.Module.Manager",
                    "Catalog.Items.ManagerModule",
                    "Catalogs/Items/Ext/ManagerModule.bsl",
                ),
                (
                    "Report.ParityReport.Form.MainForm.Module.Form",
                    "Report.ParityReport.Form.MainForm.FormModule",
                    "Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl",
                ),
            ] {
                for tail in ["", ".Body"] {
                    assert_mapping(
                        &authority,
                        &fixture.context,
                        &format!("main:{at}{tail}"),
                        expected,
                        &fixture.source.join(relative),
                    );
                }
            }
        }
    }

    struct AnalysisFixture(DiagnosticProviderOutcome);

    impl DiagnosticProvider for AnalysisFixture {
        fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
            static DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
                id: BSL_ANALYZER_PROVIDER,
                actions: &[DiagnosticAction::Analyze],
                findings_target_kinds: &[TargetKind::Module],
                emits_focus_kinds: &[DiagnosticFocusKind::Target],
            };
            &DESCRIPTOR
        }
        fn execute(
            &self,
            request: &DiagnosticProviderRequest,
            context: &DiagnosticContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> DiagnosticProviderOutcome {
            assert_eq!(request.action, DiagnosticAction::Analyze);
            assert_eq!(context.target.target_kind, TargetKind::Module);
            self.0.clone()
        }
    }

    #[test]
    fn analyze_filters_neighbors_after_mapping_but_keeps_outside_and_incomplete_failures() {
        let fixture = RealExternalReaderFixture::new();
        let authority = fixture
            .operation_read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
        let mapping = authority
            .diagnostic_mapping(
                &QualifiedAddress::parse(
                    "artifact_processor:ExternalDataProcessor.Import.Module.Object",
                )
                .unwrap(),
                &fixture.context,
            )
            .unwrap();
        let selected = fixture.processor.join("Import/Ext/ObjectModule.bsl");
        let neighbor = fixture.processor.join("Импорт/Ext/ObjectModule.bsl");
        for scenario in [
            "neighbor",
            "outside",
            "incomplete",
            "neighbor_only",
            "incomplete_empty",
        ] {
            let mut observations = vec![
                finding(&selected, "Selected"),
                finding(&neighbor, "Neighbor"),
            ];
            if scenario == "neighbor_only" {
                observations = vec![finding(&neighbor, "Neighbor")];
            } else if scenario == "incomplete_empty" {
                observations.clear();
            }
            let complete = !matches!(scenario, "incomplete" | "incomplete_empty");
            if scenario == "outside" {
                observations.push(finding(
                    &fixture.report.join("Sales/Ext/ObjectModule.bsl"),
                    "Outside",
                ));
            }
            let outcome = DiagnosticProviderOutcome {
                status: DiagnosticProviderStatus::Completed,
                complete,
                version: None,
                observations,
                rules: Vec::new(),
                readiness: None,
                error: None,
                suppressions: Vec::new(),
            };
            let registry =
                DiagnosticProviderRegistry::new(vec![Arc::new(AnalysisFixture(outcome))]).unwrap();
            let result = DiagnosticCoordinator::new(registry, &mapping)
                .execute(&request(&mapping), &fixture.context, &fixture.cancellation)
                .unwrap();
            if scenario == "outside" {
                assert!(!result.ok);
                assert!(!result.complete);
                assert_eq!(result.providers[0].status, DiagnosticProviderStatus::Failed);
                assert!(
                    result.items.is_empty(),
                    "outside resource must poison the whole provider before filtering"
                );
            } else {
                let has_selected = !matches!(scenario, "neighbor_only" | "incomplete_empty");
                assert_eq!(
                    result.items.len(),
                    usize::from(has_selected),
                    "{scenario}: only selected module findings may remain"
                );
                if has_selected {
                    assert!(
                        matches!(&result.items[0], DiagnosticItem::Diagnostic { code, .. } if code == "Selected")
                    );
                }
                assert_eq!(result.complete, complete);
                assert_eq!(
                    result.state,
                    if !complete {
                        DiagnosticResultState::Partial
                    } else {
                        DiagnosticResultState::Completed
                    }
                );
            }
        }
    }

    #[test]
    fn diagnostic_mapping_refuses_an_unregistered_form_even_with_a_module_file() {
        let fixture = RealExternalReaderFixture::new();
        write(
            &fixture.processor.join("Import/Forms/Orphan.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>Orphan</Name></Properties></Form></MetaDataObject>"#,
        );
        write(
            &fixture
                .processor
                .join("Import/Forms/Orphan/Ext/Form/Module.bsl"),
            "Procedure Run()\nEndProcedure\n",
        );
        let authority = fixture
            .operation_read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
        let error = authority
            .diagnostic_mapping(
                &QualifiedAddress::parse(
                    "artifact_processor:ExternalDataProcessor.Import.Form.Orphan.Module.Form",
                )
                .unwrap(),
                &fixture.context,
            )
            .err()
            .expect("orphan form must not acquire diagnostic authority");
        assert_eq!(error.code(), RefusalCode::NotFound);
    }
    #[test]
    fn diagnostic_mapping_refuses_replacement_of_its_retained_source_root() {
        if !supports_retained_root_replacement_test() {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::RenameUnderOpenHandle,
                "the platform keeps an open handle from being renamed or replaced",
            );
            return;
        }
        let fixture = RealReaderFixture::new();
        let authority = fixture.operation_read_authority();
        let saved = fixture.root.path().join("retained-a");
        fs::rename(&fixture.source, &saved).unwrap();
        fs::create_dir_all(&fixture.source).unwrap();
        fs::copy(
            saved.join("Configuration.xml"),
            fixture.source.join("Configuration.xml"),
        )
        .unwrap();
        let error = authority
            .diagnostic_mapping(
                &QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap(),
                &fixture.context,
            )
            .err()
            .expect("changed named root must not acquire diagnostic authority");
        assert_eq!(error.code(), RefusalCode::ProviderUnavailable);
    }
}

fn external_leaf_descriptor(kind: &str, name: &str, comment_bytes: usize) -> String {
    format!(
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind} uuid="10000000-0000-4000-8000-000000000010"><Properties><Name>{name}</Name></Properties><ChildObjects/></{kind}><!--{}--></MetaDataObject>"#,
        "x".repeat(comment_bytes)
    )
}

fn external_leaf_fixture(kind: SourceSetKind) -> (tempfile::TempDir, PathBuf) {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("external");
    fs::create_dir(&source).unwrap();
    let source_type = if kind == SourceSetKind::ExternalProcessor {
        "EXTERNAL_DATA_PROCESSORS"
    } else {
        "EXTERNAL_REPORTS"
    };
    fs::write(workspace.path().join("v8project.yaml"), format!("format: DESIGNER\nsource-set:\n  - name: artifact\n    type: {source_type}\n    path: external\n")).unwrap();
    let source = fs::canonicalize(source).unwrap();
    (workspace, source)
}

fn external_leaf_read(source: &Path, kind: SourceSetKind) -> ProviderReadAuthority {
    ProviderReadAuthority::new_local_snapshot(
        "artifact",
        "external-leaf-identity",
        kind,
        Arc::new(unica_workspace::read::open_source_directory(source).unwrap()),
        "external-leaf-local-read".into(),
    )
}

fn nested_check_fixture() -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Subsystems/Sales.xml"),
        &fixture_text("acceptance/workspace-nested-check/src/Subsystems/Parent.xml")
            .replace("Parent", "Sales")
            .replace(
                "<Subsystem>Child</Subsystem>",
                "<Subsystem>Orders</Subsystem>",
            ),
    );
    write(&fixture.source.join("Subsystems/Sales/Subsystems/Orders.xml"),
        &fixture_text("acceptance/workspace-nested-check/src/Subsystems/Parent/Subsystems/Child/Subsystems/Grandchild.xml")
            .replace("Grandchild", "Orders"));
    fixture
}

fn install_xdto_check_fixture(fixture: &RealReaderFixture) {
    let path = fixture.source.join("Configuration.xml");
    let config=fs::read_to_string(&path).unwrap().replace("</ChildObjects>","<XDTOPackage>Control</XDTOPackage><XDTOPackage>Unsupported</XDTOPackage><XDTOPackage>Paged</XDTOPackage><XDTOPackage>Mixed</XDTOPackage></ChildObjects>");
    write(&path, &config);
    for name in ["Control", "Unsupported", "Paged", "Mixed"] {
        for suffix in [".xml", "/Ext/Package.bin"] {
            let relative = format!("XDTOPackages/{name}{suffix}");
            write(
                &fixture.source.join(&relative),
                &fixture_text(&format!("acceptance/workspace-xdto-check/src/{relative}")),
            );
        }
    }
}

fn install_common_template_fixture(fixture: &RealReaderFixture) {
    let config_path = fixture.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap().replace("</ChildObjects>", "<CommonTemplate>Print</CommonTemplate><CommonTemplate>Schema</CommonTemplate><CommonTemplate>Html</CommonTemplate></ChildObjects>");
    write(&config_path, &config);
    for name in ["Print", "Schema", "Html"] {
        for suffix in [".xml", "/Ext/Template.xml"] {
            let relative = format!("CommonTemplates/{name}{suffix}");
            write(
                &fixture.source.join(&relative),
                &fixture_text(&format!(
                    "acceptance/workspace-common-template/src/{relative}"
                )),
            );
        }
    }
}
