use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use unica_coder::application::UnicaApplication;

#[test]
fn project_health_parent_repository_reports_repository_relative_remediation() {
    let root = temp_root("parent-repository");
    git(&root, &["init"]);
    let workspace = root.join("workspace");
    create_platform_workspace(&workspace, "src");
    fs::write(
        workspace.join("src/ConfigDumpInfo.xml"),
        "<ConfigDumpInfo/>\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitattributes"),
        "*.xml text eol=lf\n*.bsl text eol=lf\n*.bin -text\nXDTOPackages/**/Ext/Package.bin text eol=lf\n",
    )
    .unwrap();
    git(
        &root,
        &[
            "add",
            ".gitignore",
            ".gitattributes",
            "workspace/v8project.yaml",
            "workspace/src/Configuration.xml",
        ],
    );
    git(&root, &["add", "-f", "workspace/src/ConfigDumpInfo.xml"]);

    let result = status(&workspace);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    let diagnostic = data["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "git.runtime_sidecar_tracked")
        .expect("runtime sidecar diagnostic");
    assert_eq!(
        diagnostic["remediation"]["commands"][0]["cwd"],
        root.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(
        diagnostic["remediation"]["commands"][0]["args"][3],
        "workspace/src/ConfigDumpInfo.xml"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_full_portable_repository_is_ready() {
    let root = temp_root("full-ready");
    git(&root, &["init"]);
    create_platform_workspace(&root, "src");
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitattributes"),
        "*.xml text eol=lf\n*.bsl text eol=lf\n*.bin -text\nXDTOPackages/**/Ext/Package.bin text eol=lf\n",
    )
    .unwrap();
    git(
        &root,
        &[
            "add",
            ".gitignore",
            ".gitattributes",
            "v8project.yaml",
            "src/Configuration.xml",
        ],
    );
    let before = snapshot_files(&root);

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_eq!(data["ready"], true);
    assert_eq!(data["repositoryReady"], true, "{data}");
    assert_eq!(snapshot_files(&root), before);
    assert!(!root.join(".build/unica/services").exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn project_health_linked_source_route_is_reported_without_following_it() {
    use std::os::unix::fs::symlink;

    let root = temp_root("linked-source");
    git(&root, &["init"]);
    fs::create_dir_all(root.join("real-src")).unwrap();
    fs::write(
        root.join("real-src/Configuration.xml"),
        "<MetaDataObject/>\n",
    )
    .unwrap();
    symlink(root.join("real-src"), root.join("src-link")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src-link\n",
    )
    .unwrap();

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_eq!(data["ready"], false);
    assert!(data["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| { diagnostic["code"] == "source_set.path_unsafe" }));
    let _ = fs::remove_dir_all(root);
}

fn status(workspace: &Path) -> unica_coder::application::OperationResult {
    let mut args = Map::new();
    args.insert("cwd".into(), Value::String(workspace.display().to_string()));
    UnicaApplication::new()
        .call_tool("unica.project.status", &args)
        .unwrap()
}

fn create_platform_workspace(root: &Path, source_path: &str) {
    fs::create_dir_all(root.join(source_path)).unwrap();
    fs::write(
        root.join(source_path).join("Configuration.xml"),
        "<MetaDataObject/>\n",
    )
    .unwrap();
    fs::write(
        root.join("v8project.yaml"),
        format!(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: {source_path}\n"
        ),
    )
    .unwrap();
}

fn snapshot_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn collect(root: &Path, current: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.file_name().and_then(|name| name.to_str()) == Some(".git") {
                continue;
            }
            if path.is_dir() {
                collect(root, &path, files);
            } else {
                files.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }
    let mut files = Vec::new();
    collect(root, root, &mut files);
    files
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "unica-platform-project-health-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
