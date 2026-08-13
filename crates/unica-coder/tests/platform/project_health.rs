use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
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
        diagnostic["remediation"]["commands"][0]["argv"][3],
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
    let git_before = snapshot_files(&root.join(".git"));

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_eq!(data["ready"], true);
    assert_eq!(data["repositoryReady"], true, "{data}");
    assert_eq!(snapshot_files(&root), before);
    assert_eq!(snapshot_files(&root.join(".git")), git_before);
    assert!(!root.join(".build/unica/services").exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn project_health_does_not_execute_configured_fsmonitor_hook() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp_root("fsmonitor-disabled");
    git(&root, &["init"]);
    create_platform_workspace(&root, "src");
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&root, &["add", "."]);
    let marker = root.join("hook-ran");
    let hook = root.join("fsmonitor-hook.sh");
    fs::write(
        &hook,
        format!("#!/bin/sh\nprintf ran >> '{}'\nprintf '0\\n'\n", marker.display()),
    )
    .unwrap();
    let mut permissions = fs::metadata(&hook).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&hook, permissions).unwrap();
    git(
        &root,
        &["config", "core.fsmonitor", hook.to_str().unwrap()],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    assert!(!marker.exists(), "fsmonitor hook was executed by project health");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_full_portable_linked_worktree_is_ready_and_read_only() {
    let root = temp_root("linked-worktree");
    let repository = root.join("repository");
    fs::create_dir_all(&repository).unwrap();
    git(&repository, &["init"]);
    create_platform_workspace(&repository, "src");
    fs::write(
        repository.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(
        repository.join(".gitattributes"),
        "*.xml text eol=lf\n*.bsl text eol=lf\n*.bin -text\nXDTOPackages/**/Ext/Package.bin text eol=lf\n",
    )
    .unwrap();
    git(&repository, &["add", "."]);
    git(
        &repository,
        &[
            "-c",
            "user.name=Unica Test",
            "-c",
            "user.email=unica@example.test",
            "commit",
            "-m",
            "fixture",
        ],
    );
    let linked = root.join("linked");
    git(
        &repository,
        &["worktree", "add", "--detach", linked.to_str().unwrap(), "HEAD"],
    );
    let before = snapshot_files(&linked);
    let git_before = snapshot_files(&repository.join(".git"));

    let result = status(&linked);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_eq!(data["ready"], true, "{data}");
    assert_eq!(data["repositoryReady"], true, "{data}");
    assert_eq!(snapshot_files(&linked), before);
    assert_eq!(snapshot_files(&repository.join(".git")), git_before);
    assert!(!linked.join(".build/unica/services").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_handles_real_index_with_43k_sibling_paths() {
    let root = temp_root("large-index");
    git(&root, &["init"]);
    let workspace = root.join("workspace");
    create_platform_workspace(&workspace, "src");
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
    let oid = git_with_input(&root, &["hash-object", "-w", "--stdin"], b"fixture\n");
    let mut index_info = Vec::with_capacity(43_000 * 80);
    for index in 0..43_000 {
        write!(
            index_info,
            "100644 {}\tlarge-sibling/{index:05}.txt\0",
            oid.trim()
        )
        .unwrap();
    }
    git_with_input(&root, &["update-index", "-z", "--index-info"], &index_info);
    let staged_size = git_output(&root, &["ls-files", "--cached", "--stage", "-z"])
        .stdout
        .len();
    assert!(staged_size > 1024 * 1024, "staged output={staged_size}");

    let result = status(&workspace);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_ne!(
        data["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["id"] == "repository.index")
            .unwrap()["status"],
        "notRun",
        "{data}"
    );
    assert!(!data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.inspection_incomplete"
            && diagnostic["evidence"].as_array().is_some_and(|evidence| {
                evidence.iter().any(|item| item.as_str().is_some_and(|text| text.contains("truncated")))
            })
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn project_health_inspects_unix_source_path_with_literal_backslash() {
    let root = temp_root("literal-backslash-source");
    git(&root, &["init"]);
    create_platform_workspace(&root, "src\\name");
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(root.join("src\\name/Bad.xml"), "<A/>\r\n<B/>\r\n").unwrap();
    git(
        &root,
        &[
            "add",
            ".gitignore",
            "v8project.yaml",
            "src\\name/Configuration.xml",
            "src\\name/Bad.xml",
        ],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_eq!(data["repositoryReady"], false, "{data}");
    assert!(data["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "git.text_policy_missing"));
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
    let output = git_output(cwd, args);
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

fn git_with_input(cwd: &Path, args: &[&str], input: &[u8]) -> String {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
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
