//! apply reads the configuration whole: no file or namespace size of its own
//! refuses a correct call (#1119, PR7).

use super::actor_capacity_tests::direct_v5;
use super::*;
use unica_execution::invocation_store::ToolIdentity;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use unica_execution_contract::invocation::DomainResult;

/// The former per-file bound of the reference scan of `object.remove`.
const FORMER_REMOVE_FILE_BOUND: usize = 16 * 1024 * 1024;
/// The former per-file bound of apply staging.
const FORMER_STAGING_FILE_BOUND: usize = 32 * 1024 * 1024;

struct Fixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    runtime: V5CanonicalInvocationRuntime,
}

impl Fixture {
    /// The empty 8.3.27 configuration with the given objects.
    fn new(objects: &[(&str, &str)]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            unica_workspace::test_support::canonical_path(root.path()).join("workspace");
        let image = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/platform_8_3_27/empty_configuration");
        for (relative, bytes) in unica_workspace::test_support::tree_snapshot(&image) {
            let path = workspace.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let fixture = Self {
            _root: root,
            workspace,
            runtime: V5CanonicalInvocationRuntime::new(
                Arc::new(
                    crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
                ),
                Arc::new(TokioClock),
            ),
        };
        let ops = objects
            .iter()
            .map(|(kind, name)| {
                json!({"op":"object.create","args":{"values":{"kind":kind,"name":name}}})
            })
            .collect::<Vec<_>>();
        fixture.publish(json!({"at":"main:Configuration","ops":ops}));
        fixture
    }

    fn call(&self, tool: ToolIdentity, args: Value) -> DomainResult {
        direct_v5(
            &self.runtime,
            InvocationRequest::new(tool, args, self.workspace.to_string_lossy(), 7_000).unwrap(),
        )
        .unwrap()
    }

    fn publish(&self, args: Value) -> DomainResult {
        let planned = self.call(ToolIdentity::Apply, args);
        assert!(planned.ok, "{planned:?}");
        let token = planned.data.as_ref().unwrap()["executionToken"].clone();
        let published = self.call(ToolIdentity::Apply, json!({"executionToken": token}));
        assert!(published.ok, "{published:?}");
        published
    }
}

/// Lines of a BSL comment until the text is larger than `bytes`.
fn comment_padding(bytes: usize, line: &str) -> String {
    let mut text = String::with_capacity(bytes + line.len());
    while text.len() <= bytes {
        text.push_str(line);
    }
    text
}

/// The reference scan of `object.remove` reads every module whole: a module
/// larger than its former 16 MiB bound does not refuse the removal.
#[test]
fn object_remove_scans_a_module_larger_than_the_former_bound() {
    let fixture = Fixture::new(&[("Catalog", "Товары"), ("CommonModule", "Большой")]);
    let module = fixture
        .workspace
        .join("src/CommonModules/Большой/Ext/Module.bsl");
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::write(
        &module,
        comment_padding(
            FORMER_REMOVE_FILE_BOUND,
            "// строка без ссылок на объекты\r\n",
        ),
    )
    .unwrap();

    fixture.publish(json!({"at": "main:Catalog.Товары", "ops": [{"op": "object.remove"}]}));
    assert!(!fixture.workspace.join("src/Catalogs/Товары.xml").exists());
    assert!(std::fs::metadata(&module).unwrap().len() > FORMER_REMOVE_FILE_BOUND as u64);
}

/// apply stages a descriptor larger than its former 32 MiB bound and edits it.
#[test]
fn apply_edits_a_descriptor_larger_than_the_former_staging_bound() {
    let fixture = Fixture::new(&[("Catalog", "Товары")]);
    let descriptor = fixture.workspace.join("src/Catalogs/Товары.xml");
    let text = std::fs::read_to_string(&descriptor).unwrap();
    let padding = comment_padding(
        FORMER_STAGING_FILE_BOUND,
        "<!-- заполнитель большого описания -->\r\n",
    );
    let padded = text.replacen(
        "</MetaDataObject>",
        &format!("{padding}</MetaDataObject>"),
        1,
    );
    std::fs::write(&descriptor, &padded).unwrap();

    fixture.publish(json!({"at": "main:Catalog.Товары", "ops": [
        {"op": "props.set", "args": {"values": {"Comment": "Большой справочник"}}}
    ]}));
    let written = std::fs::read_to_string(&descriptor).unwrap();
    assert!(written.contains("Большой справочник"));
    assert!(written.len() > FORMER_STAGING_FILE_BOUND);
}
