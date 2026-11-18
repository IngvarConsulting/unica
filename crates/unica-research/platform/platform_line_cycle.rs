//! Full cycle on an installed platform line, through the product adapters.
//!
//! The reference `all_kinds` export of the line's format is loaded into a new
//! file infobase and exported back by the runner's full dump: the export must
//! equal the reference. Then the product edits those sources over MCP, as an
//! agent would (a new catalog, an enum value, a list form and, on a line after
//! 8.3.27, a root property of that line), the edit is built into the infobase
//! and exported again: the platform must write exactly the bytes the product
//! wrote. That export is packaged into a `.cf`, loaded into a second infobase
//! and exported once more, with the same bytes. `ConfigDumpInfo.xml` is never
//! compared: the reference does not keep it.
//!
//! A research test on a real platform: it is ignored in ordinary runs and runs
//! by hand through `scripts/dev/run-research.py --profile platform`, which runs
//! ignored tests. `UNICA_PLATFORM_LINE` names a line the format profile measured
//! (`EXPORT_FORMAT_PLATFORM_LINES`) and that has an `all_kinds` reference. Then
//! `V8_RUNNER` must name the v8-runner executable of the pinned version
//! (`plugins/unica/third-party/tools.lock.json`) and `UNICA_BINARY` the built
//! product binary that edits the sources; `UNICA_PLATFORM_BUILD` picks an exact
//! build instead of the newest installed one of the line.
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use unica_adapter_v8_runner::adapter::{RuntimeAdapter, RuntimeEnvironment};
use unica_adapter_v8_runner::full_dump::default_platform_roots;
use unica_artifact_delivery::engine::BundledTool;
use unica_execution::runner::attempt::RuntimeWorkspaceObservation;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_process::runner::system_process_runner;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::format_profile::EXPORT_FORMAT_PLATFORM_LINES;

const LINE_ENV: &str = "UNICA_PLATFORM_LINE";
const BUILD_ENV: &str = "UNICA_PLATFORM_BUILD";
const RUNNER_ENV: &str = "V8_RUNNER";
const BINARY_ENV: &str = "UNICA_BINARY";
/// Files the reference export does not keep.
const NOT_IN_REFERENCE: &[&str] = &["ConfigDumpInfo.xml"];

/// Export format written by `line`, from the format profile's measured lines.
fn line_format(line: &str) -> Option<&'static str> {
    EXPORT_FORMAT_PLATFORM_LINES
        .iter()
        .find(|(_, writer)| *writer == line)
        .map(|(format, _)| *format)
}

fn plugin_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/unica")
}

fn reference_export(format: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/formats/platform_xml")
        .join(format)
        .join("all_kinds/src")
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
            .or_else(|| Some(plugin_root()))
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
        RuntimeAdapter::with_runner(system_process_runner(), &self.environment)
            .invoke("unica.runtime.execute", &args, context, false, true)
            .expect("full dump")
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

/// One MCP session of the product binary over stdio, in one workspace.
struct Product {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: PathBuf,
    next_id: u64,
}

impl Product {
    fn start(binary: &Path, workspace: &Path, state: &Path) -> Self {
        std::fs::create_dir_all(state).unwrap();
        let stderr = state.join("unica-stderr.log");
        let mut child = Command::new(binary)
            .current_dir(workspace)
            .env("UNICA_PROVIDER_STATE_DIR", state)
            .env("UNICA_DAEMON_IDLE_GRACE_MS", "5000")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap_or_else(|error| panic!("start {}: {error}", binary.display()));
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut product = Self {
            child,
            stdin,
            stdout,
            stderr,
            next_id: 0,
        };
        product.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "platform-line-cycle", "version": "0"}}),
        );
        product.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        product
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("session is open");
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).unwrap();
            assert!(
                read > 0,
                "unica closed the session during {method}: {}",
                std::fs::read_to_string(&self.stderr).unwrap_or_default()
            );
            let message: Value = serde_json::from_str(&line)
                .unwrap_or_else(|error| panic!("unica wrote no JSON ({error}): {line}"));
            if message.get("id") == Some(&json!(id)) {
                return message;
            }
        }
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let response = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = response["result"]["structuredContent"].clone();
        assert!(
            result["ok"] == json!(true),
            "{tool} {arguments}: {response:#}"
        );
        result
    }

    /// Plan `ops` at `at` and execute the plan, as an agent does.
    fn apply(&mut self, at: &str, ops: Value) {
        let planned = self.call("unica.apply", json!({"at": at, "ops": ops}));
        let token = planned["data"]["executionToken"]
            .as_str()
            .unwrap_or_else(|| panic!("plan at {at} has no execution token: {planned:#}"))
            .to_string();
        self.call("unica.apply", json!({"executionToken": token}));
    }
}

impl Drop for Product {
    fn drop(&mut self) {
        // Closing stdin ends the session.
        self.stdin.take();
        let _ = self.child.wait();
    }
}

/// The edits an agent makes in the sources exported as `reference`: each
/// creates what the formats write differently, so the platform's export
/// proves the product's bytes.
fn edits(reference: &Path) -> Vec<(&'static str, Value)> {
    // A root property that only some lines have: it is set where the line's
    // own export of the configuration root carries it.
    let configuration = std::fs::read(reference.join("Configuration.xml")).unwrap();
    let root = if configuration
        .windows(b"<ClientApplicationTheme>".len())
        .any(|window| window == b"<ClientApplicationTheme>")
    {
        json!({"ClientApplicationTheme": "Dark"})
    } else {
        json!({"Comment": "Правка цикла"})
    };
    vec![
        (
            "main:Configuration",
            json!([{"op": "object.create",
                    "args": {"values": {"kind": "Catalog", "name": "ПравкаЦикла"}}}]),
        ),
        (
            "main:Enum.СпособыУстановкиКурсаВалюты",
            json!([{"op": "enumValue.add", "args": {"items": [{"name": "ПравкаЦикла"}]}}]),
        ),
        (
            "main:Catalog.Валюты",
            json!([{"op": "form.add",
                    "args": {"items": [{"name": "ФормаПравкиЦикла", "type": "ListForm"}]}}]),
        ),
        (
            "main:Configuration",
            json!([{"op": "props.set", "args": {"values": root}}]),
        ),
    ]
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
#[ignore = "needs an installed 1C platform: set UNICA_PLATFORM_LINE, V8_RUNNER and UNICA_BINARY and run scripts/dev/run-research.py --profile platform"]
fn platform_line_full_cycle_keeps_the_reference_export() {
    let line = std::env::var(LINE_ENV)
        .unwrap_or_else(|_| panic!("{LINE_ENV} must name the platform line of the cycle"));
    let format = line_format(&line)
        .unwrap_or_else(|| panic!("{LINE_ENV}={line}: no reference export for this line"));
    let program = PathBuf::from(
        std::env::var_os(RUNNER_ENV)
            .unwrap_or_else(|| panic!("{RUNNER_ENV} must name v8-runner when {LINE_ENV} is set")),
    );
    let binary = PathBuf::from(
        std::env::var_os(BINARY_ENV)
            .unwrap_or_else(|| panic!("{BINARY_ENV} must name the built unica binary")),
    );
    let build = std::env::var(BUILD_ENV).unwrap_or_else(|_| newest_build(&line));
    assert!(
        build.starts_with(&format!("{line}.")),
        "{BUILD_ENV}={build} is not a build of {line}"
    );
    let reference = reference_export(format);
    let cycle = Cycle {
        environment: HostRunnerEnvironment { program },
        build,
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "unica-platform-line-cycle-{line}-{}-{stamp}",
        std::process::id()
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

    // The product edits the sources; the platform builds the edit and
    // exports exactly the bytes the product wrote.
    {
        let mut product = Product::start(&binary, &first.cwd, &root.join("product-state"));
        for (at, ops) in edits(&reference) {
            product.apply(at, ops);
        }
    }
    // A write through the product moves the workspace epoch; the build is
    // admitted for the workspace as it is now.
    let first = {
        let current = cycle
            .environment
            .workspace()
            .discover(&first.cwd)
            .expect("rediscover");
        WorkspaceContext {
            workspace_epoch: current.workspace_epoch,
            ..first
        }
    };
    let edited = root.join("edited");
    copy_tree(&first_export, &edited);
    assert!(
        !tree_differences(&reference, &edited).is_empty(),
        "{line}: the product edit changed nothing"
    );
    assert_ok(
        "build of the edit",
        &cycle.runtime(
            &first,
            &[("operation", "build".into()), ("sourceSet", "main".into())],
        ),
    );
    assert_ok("full dump of the edit", &cycle.full_dump(&first));
    let differences = tree_differences(&edited, &first_export);
    assert!(
        differences.is_empty(),
        "{line}: the platform exports the product's edit differently: {differences:#?}"
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
