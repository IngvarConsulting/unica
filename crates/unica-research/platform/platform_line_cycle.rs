//! Full cycle on an installed platform line, through the product adapters.
//!
//! The reference `all_kinds` export of the line's format is loaded into a new
//! file infobase, exported back by the verified full dump (which checks the
//! export with the Designer semantics), packaged into a `.cf`, loaded into a
//! second infobase and exported again. Both exports must equal the reference
//! byte for byte, apart from `ConfigDumpInfo.xml`, which the reference does not
//! keep.
//!
//! A research test on a real platform: it runs only by hand, through
//! `scripts/dev/run-research.py --profile platform`, and only when
//! `UNICA_PLATFORM_LINE` names a line (`8.3.27`, `8.5.1`, `8.5.4`). Then
//! `V8_RUNNER` must name the v8-runner executable of the pinned version
//! (`plugins/unica/third-party/tools.lock.json`); `UNICA_PLATFORM_BUILD` picks
//! an exact build instead of the newest installed one of the line.
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use unica_adapter_v8_runner::adapter::{RuntimeAdapter, RuntimeEnvironment};
use unica_adapter_v8_runner::full_dump::{
    default_platform_roots, FullDumpInvocation, SystemPlatformResolver, VerifiedFullDumpAdapter,
};
use unica_artifact_delivery::engine::BundledTool;
use unica_execution::runner::attempt::RuntimeWorkspaceObservation;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_process::runner::system_process_runner;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;

const LINE_ENV: &str = "UNICA_PLATFORM_LINE";
const BUILD_ENV: &str = "UNICA_PLATFORM_BUILD";
const RUNNER_ENV: &str = "V8_RUNNER";
/// Files the reference export does not keep.
const NOT_IN_REFERENCE: &[&str] = &["ConfigDumpInfo.xml"];

/// Export format written by each platform line the corpus has a reference for.
fn line_format(line: &str) -> Option<&'static str> {
    match line {
        "8.3.27" => Some("2.20"),
        "8.5.1" => Some("2.21"),
        "8.5.4" => Some("2.22"),
        _ => None,
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn newest_build(line: &str) -> String {
    let prefix = format!("{line}.");
    let mut builds: Vec<Vec<u32>> = default_platform_roots()
        .iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flatten()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with(&prefix))
        .filter_map(|name| name.split('.').map(|p| p.parse().ok()).collect())
        .collect();
    builds.sort();
    let build = builds
        .pop()
        .unwrap_or_else(|| panic!("no installed build of platform line {line}"));
    build
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// The runtime environment of the product with v8-runner taken from the host.
struct HostRunnerEnvironment {
    program: PathBuf,
}

impl RuntimeEnvironment for HostRunnerEnvironment {
    fn workspace(&self) -> &dyn RuntimeWorkspaceObservation {
        unica_coder::infrastructure::runtime_composition::runtime_environment().workspace()
    }
    fn plugin_root(&self, cwd: &Path) -> Option<PathBuf> {
        unica_coder::infrastructure::runtime_composition::runtime_environment()
            .plugin_root(cwd)
            .or_else(|| Some(repo_root().join("plugins/unica")))
    }
    fn resolve_v8_tool(
        &self,
        _plugin_root: &Path,
        _require_exists: bool,
    ) -> Result<BundledTool, String> {
        Ok(BundledTool {
            program: self.program.clone(),
            warnings: Vec::new(),
            missing: None,
        })
    }
    fn preview(
        &self,
        tool_name: &str,
        label: &str,
        mutating: bool,
        tool: BundledTool,
        command: Vec<String>,
    ) -> AdapterOutcome {
        unica_coder::infrastructure::runtime_composition::runtime_environment()
            .preview(tool_name, label, mutating, tool, command)
    }
}

struct Cycle {
    environment: HostRunnerEnvironment,
    build: String,
}

impl Cycle {
    fn ibcmd(&self) -> PathBuf {
        default_platform_roots()
            .into_iter()
            .flat_map(|root| {
                let build = root.join(&self.build);
                [
                    build.join("bin").join("ibcmd.exe"),
                    build.join("bin").join("ibcmd"),
                    build.join("ibcmd"),
                ]
            })
            .find(|path| path.is_file())
            .unwrap_or_else(|| panic!("no ibcmd in platform build {}", self.build))
    }

    fn workspace(&self, root: &Path, name: &str, sources: &Path) -> WorkspaceContext {
        let workspace = root.join(name);
        copy_tree(sources, &workspace.join("src"));
        let infobase = workspace.join("ib");
        std::fs::write(
            workspace.join("v8project.yaml"),
            format!(
                "workPath: .work\nformat: DESIGNER\nproviders:\n  push: designer\n  pull: designer\ninfobase:\n  connection: '/F {}'\ntools:\n  platform:\n    version: '{}'\n    strict: true\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                infobase.display(),
                self.build
            ),
        )
        .unwrap();
        WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace,
            cache_root: root.join(format!("{name}-cache")),
            workspace_epoch: 0,
        }
    }

    fn runtime(&self, context: &WorkspaceContext, args: &[(&str, Value)]) -> AdapterOutcome {
        let args: Map<String, Value> = args
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect();
        RuntimeAdapter::with_runner(system_process_runner(), &self.environment)
            .invoke("unica.runtime.execute", &args, context, false, true)
            .unwrap_or_else(|error| panic!("{args:?}: {error}"))
    }

    fn full_dump(&self, context: &WorkspaceContext) -> AdapterOutcome {
        let args = Map::from_iter([
            ("dryRun".to_string(), Value::Bool(false)),
            ("operation".to_string(), Value::String("dump".to_string())),
            ("mode".to_string(), Value::String("full".to_string())),
        ]);
        VerifiedFullDumpAdapter::with_dependencies(
            system_process_runner(),
            &SystemPlatformResolver,
            self.environment.program.clone(),
            &self.environment,
            unica_coder::infrastructure::runtime_composition::full_dump_semantics(),
        )
        .invoke(
            "unica.runtime.execute",
            FullDumpInvocation::RuntimeExecute,
            &args,
            context,
            &CancellationToken::new(),
        )
        .expect("verified full dump")
    }

    /// Create the infobase of the workspace from its sources.
    ///
    /// The product has no operation that creates an infobase, and its push
    /// (`build`) refuses an infobase the working copy has no memory of, so
    /// v8-runner creates it from the sources directly.
    fn create_from_sources(&self, context: &WorkspaceContext) {
        let created = std::process::Command::new(&self.environment.program)
            .args(["--no-color", "infobase", "create"])
            .current_dir(&context.cwd)
            .output()
            .expect("v8-runner infobase create");
        assert!(
            created.status.success(),
            "v8-runner infobase create: {}{}",
            String::from_utf8_lossy(&created.stdout),
            String::from_utf8_lossy(&created.stderr)
        );
    }

    /// Create an empty infobase with ibcmd of the build and load the package
    /// into it through the product's `load` operation.
    fn load_package(&self, context: &WorkspaceContext, package: &Path) {
        let infobase = context.cwd.join("ib");
        let created = std::process::Command::new(self.ibcmd())
            .arg("infobase")
            .arg("create")
            .arg(format!("--db-path={}", infobase.display()))
            .output()
            .expect("ibcmd infobase create");
        assert!(
            created.status.success(),
            "ibcmd infobase create: {}",
            String::from_utf8_lossy(&created.stderr)
        );
        let loaded = self.runtime(
            context,
            &[
                ("operation", "load".into()),
                ("path", package.display().to_string().into()),
            ],
        );
        assert_ok("load", &loaded);
    }
}

fn assert_ok(step: &str, outcome: &AdapterOutcome) {
    assert!(
        outcome.ok,
        "{step} failed: {}\nerrors: {:?}\nstdout: {}\nstderr: {}",
        outcome.summary,
        outcome.errors,
        outcome.stdout.as_deref().unwrap_or(""),
        outcome.stderr.as_deref().unwrap_or("")
    );
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if !NOT_IN_REFERENCE.contains(&relative.as_str()) {
                out.insert(relative, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Paths that differ, are missing or are extra in `actual`.
fn tree_differences(expected: &Path, actual: &Path) -> Vec<String> {
    let (expected, actual) = (tree(expected), tree(actual));
    let mut differences = Vec::new();
    for (path, bytes) in &expected {
        match actual.get(path) {
            None => differences.push(format!("missing {path}")),
            Some(other) if other != bytes => differences.push(format!("differs {path}")),
            Some(_) => {}
        }
    }
    for path in actual.keys().filter(|path| !expected.contains_key(*path)) {
        differences.push(format!("extra {path}"));
    }
    differences
}

#[test]
fn platform_line_full_cycle_keeps_the_reference_export() {
    let Ok(line) = std::env::var(LINE_ENV) else {
        eprintln!("skipped: {LINE_ENV} names no platform line");
        return;
    };
    let format = line_format(&line)
        .unwrap_or_else(|| panic!("{LINE_ENV}={line}: no reference export for this line"));
    let program = PathBuf::from(
        std::env::var_os(RUNNER_ENV)
            .unwrap_or_else(|| panic!("{RUNNER_ENV} must name v8-runner when {LINE_ENV} is set")),
    );
    let build = std::env::var(BUILD_ENV).unwrap_or_else(|_| newest_build(&line));
    assert!(
        build.starts_with(&format!("{line}.")),
        "{BUILD_ENV}={build} is not a build of {line}"
    );
    let reference = repo_root()
        .join("tests/fixtures/formats/platform_xml")
        .join(format)
        .join("all_kinds/src");
    let cycle = Cycle {
        environment: HostRunnerEnvironment { program },
        build,
    };
    let root = std::env::temp_dir().join(format!(
        "unica-platform-line-cycle-{line}-{}",
        uuid::Uuid::new_v4()
    ));

    // Reference → infobase → verified full export.
    let first = cycle.workspace(&root, "first", &reference);
    cycle.create_from_sources(&first);
    assert_ok("first full dump", &cycle.full_dump(&first));
    let first_export = first.cwd.join("src");
    let differences = tree_differences(&reference, &first_export);
    assert!(
        differences.is_empty(),
        "{line}: export of the reference differs from it: {differences:#?}"
    );

    // Export → .cf package.
    let package = root.join("first.cf");
    assert_ok(
        "make",
        &cycle.runtime(
            &first,
            &[
                ("operation", "make".into()),
                ("sourceSet", "main".into()),
                ("output", package.display().to_string().into()),
            ],
        ),
    );
    assert!(
        std::fs::metadata(&package).is_ok_and(|meta| meta.len() > 0),
        "make wrote no package {}",
        package.display()
    );

    // Package → new infobase → export again. The second workspace starts
    // without sources, so its export comes from the loaded package only.
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let second = cycle.workspace(&root, "second", &empty);
    cycle.load_package(&second, &package);
    assert_ok("second full dump", &cycle.full_dump(&second));
    let differences = tree_differences(&first_export, &second.cwd.join("src"));
    assert!(
        differences.is_empty(),
        "{line}: export after loading the export differs: {differences:#?}"
    );

    std::fs::remove_dir_all(&root).ok();
}
