use super::*;
use crate::domain::apply::ApplyRequest;
use crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply_with_restore;
use crate::infrastructure::source_workspace::WorkspaceActor;
use std::cell::Cell;
use std::collections::BTreeMap;
use unica_execution_contract::invocation::DomainResult;
use unica_metadata_designer::support_restore::restore_support_records;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::actor::{
    PreparedApplyBatch, ProviderRootBinding, WorkspaceIdentity, WorkspaceSourceSetInput,
};
use unica_workspace::test_support::tree_snapshot;

const AT: &str = "main:DataProcessor.ТестHTML";
const MARKER: &str = "Ext/ParentConfigurations.bin";
const PAYLOAD: &str = "Ext/ParentConfigurations/РеактивныйHTML.cf";
const MODULE: &str = "DataProcessors/ТестHTML/Forms/Форма/Ext/Form/Module.bsl";

fn fixture_root() -> PathBuf {
    std::fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src"),
    )
    .unwrap()
}

fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    source: PathBuf,
    cache: PathBuf,
    actor: WorkspaceActor,
    binding: ProviderRootBinding,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("src");
        copy_tree(&fixture_root(), &source);
        std::fs::write(
            root.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set: [{name: main, type: CONFIGURATION, path: src}]\n",
        )
        .unwrap();
        let root_path = std::fs::canonicalize(root.path()).unwrap();
        let source = std::fs::canonicalize(source).unwrap();
        let cache = root_path.join(".build/unica");
        let context = WorkspaceContext {
            cwd: root_path.clone(),
            workspace_root: root_path,
            cache_root: cache.clone(),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                "main",
                &source,
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "support-restore-test",
        )
        .unwrap();
        let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
        let binding = actor.bind_provider_root("main", &source).unwrap();
        Self {
            _root: root,
            source,
            cache,
            actor,
            binding,
        }
    }

    fn prepare(
        &self,
        operations: serde_json::Value,
        supplier: &dyn RestoreSupplierRead,
    ) -> Result<PreparedApplyBatch, ApplyPlanError> {
        let args = serde_json::json!({"at":AT,"ops":operations,"dryRun":true});
        let request = ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap();
        let cancel = CancellationToken::new();
        let mut admission = self
            .actor
            .admit_apply(
                &self.binding,
                None,
                true,
                ProviderDeadline::no_deadline(),
                &cancel,
            )
            .unwrap();
        admission.bind_request_identity(
            &request.at().to_string(),
            request.ops().iter().map(|operation| {
                (
                    operation.name(),
                    operation.at().to_string(),
                    operation.args(),
                )
            }),
        );
        let (staged, effects) = plan_hidden_v13_apply_with_restore(
            &request,
            &self.binding,
            &admission,
            Some(supplier),
        )?;
        admission
            .prepare_with_effects(staged, effects)
            .map_err(|error| ApplyPlanError::staging(error, "ops"))
    }

    fn save(&self, supplier: &dyn RestoreSupplierRead) -> String {
        let prepared = self
            .prepare(serde_json::json!([{"op":"supportRule.restore"}]), supplier)
            .unwrap();
        assert_eq!(
            self.actor
                .preview_prepared_apply(&prepared)
                .unwrap()
                .commit_count_for_test(),
            0
        );
        self.actor
            .save_prepared_apply(
                prepared,
                DomainResult::success("checked support restore preview"),
            )
            .unwrap()
    }

    fn execute(&self, token: &str) -> DomainResult {
        self.actor
            .execute_saved_apply(
                token,
                ProviderDeadline::no_deadline(),
                &CancellationToken::new(),
                |_, publication| match publication {
                    Ok(result) => {
                        let mut result_out = DomainResult::success("support restore published");
                        result_out.data =
                            Some(serde_json::json!({"commits":result.commit_count_for_test()}));
                        result_out
                    }
                    Err(error) => DomainResult::canonical_rejection(
                        None,
                        unica_refusal::RefusalCode::StaleRevision,
                        error.to_string(),
                    ),
                },
            )
            .unwrap()
    }
}

struct FixtureSupplier {
    calls: Cell<usize>,
}
impl RestoreSupplierRead for FixtureSupplier {
    fn object_image(
        &self,
        payload: &[u8],
        target: &MetadataAddress,
        format: &str,
    ) -> Result<RestoreObjectImage, ApplyPlanError> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(
            payload,
            std::fs::read(fixture_root().join(PAYLOAD)).unwrap()
        );
        let mut read = PrivateObjectRead {
            root: unica_workspace::read::open_source_directory(&fixture_root()).unwrap(),
            cancellation: &CancellationToken::new(),
        };
        read_restore_object_image(&mut read, target, format)
    }
}

fn snapshot(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    if path.exists() {
        tree_snapshot(path)
    } else {
        BTreeMap::new()
    }
}

#[test]
fn saved_restore_preview_preserves_sources_and_cache_then_token_publishes_only_marker() {
    let fixture = Fixture::new();
    let supplier = FixtureSupplier {
        calls: Cell::new(0),
    };
    let sources = snapshot(&fixture.source);
    let cache = snapshot(&fixture.cache);
    let token = fixture.save(&supplier);
    assert_eq!(snapshot(&fixture.source), sources);
    assert_eq!(snapshot(&fixture.cache), cache);
    assert_eq!(supplier.calls.get(), 1);
    let result = fixture.execute(&token);
    assert!(result.ok, "{}", result.summary);
    assert_eq!(result.data.as_ref().unwrap()["commits"], 1);
    let mut after = snapshot(&fixture.source);
    let marker = after.remove(Path::new(MARKER)).unwrap();
    let mut expected = sources;
    expected.remove(Path::new(MARKER));
    assert_eq!(after, expected);
    let records = restore_support_records(&marker).unwrap();
    let object = read_restore_object_image(
        &mut PrivateObjectRead {
            root: unica_workspace::read::open_source_directory(&fixture.source).unwrap(),
            cancellation: &CancellationToken::new(),
        },
        &MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "DataProcessor.ТестHTML",
        )
        .unwrap(),
        "2.20",
    )
    .unwrap();
    assert_eq!(
        records.vendors[0]
            .objects
            .iter()
            .find(|record| record.object_uuid == object.uuid())
            .unwrap()
            .rule,
        0
    );
    let replay = fixture.execute(&token);
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::to_value(result).unwrap()
    );
    assert_eq!(
        supplier.calls.get(),
        1,
        "execution and replay must not reconvert"
    );
}

#[test]
fn saved_restore_rejects_changed_files_cf_marker_and_companion_membership() {
    for relative in [
        MODULE,
        "DataProcessors/ТестHTML.xml",
        "DataProcessors/ТестHTML/Templates/МакетСобранныйHTML/Ext/Template.bin",
        PAYLOAD,
        MARKER,
        "DataProcessors/ТестHTML/Ext/new-opaque-file.bin",
    ] {
        let fixture = Fixture::new();
        let supplier = FixtureSupplier {
            calls: Cell::new(0),
        };
        let token = fixture.save(&supplier);
        let path = fixture.source.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = std::fs::read(&path).unwrap_or_default();
        bytes.push(b'\n');
        std::fs::write(path, bytes).unwrap();
        let changed = snapshot(&fixture.source);
        let cache = snapshot(&fixture.cache);
        let result = fixture.execute(&token);
        assert!(
            !result.ok,
            "{relative} must invalidate the saved comparison"
        );
        assert_eq!(snapshot(&fixture.source), changed);
        assert_eq!(snapshot(&fixture.cache), cache);
        assert_eq!(supplier.calls.get(), 1);
    }
}

#[test]
fn unrelated_object_edit_does_not_invalidate_support_restore() {
    let fixture = Fixture::new();
    let supplier = FixtureSupplier {
        calls: Cell::new(0),
    };
    let token = fixture.save(&supplier);
    let path = fixture
        .source
        .join("CommonModules/РеактивныйКлиент/Ext/Module.bsl");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(b"\n// independent object edit\n");
    std::fs::write(&path, &bytes).unwrap();
    let result = fixture.execute(&token);
    assert!(result.ok, "{}", result.summary);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(supplier.calls.get(), 1);
}

#[test]
fn restoration_cannot_mix_with_ordinary_edits_under_any_policy() {
    for policy in ["deny", "warn", "off"] {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.source.parent().unwrap().join(".v8-project.json"),
            serde_json::json!({"editingAllowedCheck":policy}).to_string(),
        )
        .unwrap();
        let supplier = FixtureSupplier {
            calls: Cell::new(0),
        };
        let sources = snapshot(&fixture.source);
        let error = fixture.prepare(serde_json::json!([{"op":"supportRule.restore"}, {"op":"props.set", "args":{"values":{"Comment":"change"}}}]), &supplier).unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        assert!(error.to_string().contains("separate apply"), "{error}");
        assert_eq!(supplier.calls.get(), 0);
        assert_eq!(snapshot(&fixture.source), sources);
    }
}

#[test]
fn mismatch_preview_writes_nothing_while_ordinary_set_keeps_its_capability() {
    let fixture = Fixture::new();
    let path = fixture.source.join(MODULE);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.push(b'\n');
    std::fs::write(path, bytes).unwrap();
    let supplier = FixtureSupplier {
        calls: Cell::new(0),
    };
    let sources = snapshot(&fixture.source);
    let cache = snapshot(&fixture.cache);
    let error = fixture
        .prepare(serde_json::json!([{"op":"supportRule.restore"}]), &supplier)
        .unwrap_err();
    assert!(error.to_string().contains("byte-for-byte"));
    assert_eq!(snapshot(&fixture.source), sources);
    assert_eq!(snapshot(&fixture.cache), cache);
    let ordinary = fixture
        .prepare(
            serde_json::json!([{"op":"supportRule.set","args":{"values":{"rule":"locked"}}}]),
            &supplier,
        )
        .unwrap();
    assert!(fixture.actor.preview_prepared_apply(&ordinary).is_ok());
    assert_eq!(
        supplier.calls.get(),
        1,
        "ordinary set does not require comparison"
    );
}

#[derive(Clone, Copy)]
enum ProcessMode {
    Success,
    Failure,
    Unreadable,
    LaunchFailure,
    Cancel,
}

struct PackageProcess {
    mode: ProcessMode,
    private_root: std::cell::RefCell<Option<PathBuf>>,
}

impl ProcessRunner for PackageProcess {
    fn run(
        &self,
        command: &ProcessCommand,
    ) -> Result<unica_process::runner::ProcessOutput, String> {
        let root = command.cwd.parent().unwrap();
        *self.private_root.borrow_mut() = Some(root.to_path_buf());
        let argument = |key: &str| {
            PathBuf::from(
                &command.args[command.args.iter().position(|arg| arg == key).unwrap() + 1],
            )
        };
        assert_eq!(argument("--config"), command.cwd.join("v8project.yaml"));
        let output = argument("--output");
        assert_eq!(output, root.join("export"));
        let input = root.join("supplier.cf");
        assert!(command.args.contains(&input.to_string_lossy().into_owned()));
        assert_eq!(
            std::fs::read(&input).unwrap(),
            std::fs::read(fixture_root().join(PAYLOAD)).unwrap()
        );
        let primary: serde_yaml::Value =
            serde_yaml::from_slice(&std::fs::read(command.cwd.join("v8project.yaml")).unwrap())
                .unwrap();
        assert_eq!(primary["workPath"].as_str(), Some("work"));
        assert_eq!(primary["source-set"][0]["path"].as_str(), Some("seed"));
        assert!(primary.get("infobases").is_none());
        assert!(primary.get("hooks").is_none());
        let local: serde_yaml::Value = serde_yaml::from_slice(
            &std::fs::read(command.cwd.join("v8project.local.yaml")).unwrap(),
        )
        .unwrap();
        assert_eq!(local.as_mapping().unwrap().len(), 1);
        assert!(local.get("infobases").is_none());
        assert!(Path::new(local["tools"]["platform"]["path"].as_str().unwrap()).is_absolute());
        if matches!(self.mode, ProcessMode::LaunchFailure) {
            return Err("opaque supplier launch error".into());
        }
        let success = matches!(self.mode, ProcessMode::Success);
        if success {
            copy_tree(&fixture_root(), &output);
        }
        if matches!(self.mode, ProcessMode::Cancel) {
            command.cancellation.cancel();
        }
        Ok(unica_process::runner::ProcessOutput {
            status_success: success,
            status: if success { "0" } else { "4" }.into(),
            stdout: if matches!(self.mode, ProcessMode::Unreadable) {
                "opaque supplier invalid envelope".into()
            } else {
                serde_json::json!({"ok":success,"command":"convert","data":{"ok":success,"direction":"PACKAGE_TO_DESIGNER","provider_dispatched":true,"provider":{"selected":"ibcmd"},"outputs":[{"source_path":input,"target_path":output}]}}).to_string()
            },
            stderr: "opaque supplier text must never be exposed".into(),
            timed_out: false,
            cancelled: matches!(self.mode, ProcessMode::Cancel),
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        })
    }
}

#[test]
fn extraction_uses_only_private_project_and_cleans_up_on_success_failure_and_cancel() {
    for mode in [
        ProcessMode::Success,
        ProcessMode::Failure,
        ProcessMode::Unreadable,
        ProcessMode::LaunchFailure,
        ProcessMode::Cancel,
    ] {
        let fixture = Fixture::new();
        std::fs::write(fixture.source.parent().unwrap().join("v8project.local.yaml"), "workPath: must-not-be-used\ninfobases:\n  origin:\n    connection: File=must-not-be-used\nhooks: [must-not-be-used]\ntools:\n  platform:\n    path: local-platform\n    version: 8.3.27.2214\n    strict: true\n").unwrap();
        let cancellation = CancellationToken::new();
        let process = PackageProcess {
            mode,
            private_root: Default::default(),
        };
        let mut reader = SupplierReader::new(
            BundledRunner {
                tool: unica_artifact_delivery::engine::BundledTool {
                    program: PathBuf::from("controlled-test-package-converter"),
                    warnings: Vec::new(),
                    missing: None,
                },
                version: super::super::runner_015::VERSION.to_string(),
            },
            fixture.actor.context(),
            &fixture.source,
            &cancellation,
        );
        reader.process = &process;
        let workspace = snapshot(fixture.source.parent().unwrap());
        let target = MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "DataProcessor.ТестHTML",
        )
        .unwrap();
        let result = reader.object_image(
            &std::fs::read(fixture.source.join(PAYLOAD)).unwrap(),
            &target,
            "2.20",
        );
        match mode {
            ProcessMode::Success => {
                assert!(result.is_ok());
            }
            ProcessMode::Failure | ProcessMode::Unreadable | ProcessMode::LaunchFailure => {
                let error = result.unwrap_err();
                assert_eq!(error.kind(), ApplyPlanErrorKind::ProviderUnavailable);
                assert!(!error.to_string().contains("opaque supplier"));
            }
            ProcessMode::Cancel => assert_eq!(
                result.unwrap_err().kind(),
                ApplyPlanErrorKind::Staging(ApplyStagingErrorKind::Cancelled)
            ),
        }
        let private_root = process.private_root.borrow();
        assert!(
            !private_root.as_ref().unwrap().exists(),
            "private conversion must be cleaned before returning"
        );
        assert_eq!(snapshot(fixture.source.parent().unwrap()), workspace);
    }
}

#[test]
fn temp_directory_inside_sources_or_cache_refuses_before_creating_anything() {
    let fixture = Fixture::new();
    let before = snapshot(fixture.source.parent().unwrap());
    for path in [
        &fixture.source,
        &fixture.cache,
        &fixture.source.parent().unwrap().join("private-temp"),
    ] {
        assert!(isolated_temporary_root(fixture.actor.context(), &fixture.source, path).is_err());
    }
    assert_eq!(snapshot(fixture.source.parent().unwrap()), before);
}
