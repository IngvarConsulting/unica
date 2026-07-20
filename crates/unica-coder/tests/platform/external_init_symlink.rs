#[cfg(unix)]
#[test]
fn external_init_preview_rejects_symlink_components() {
    use serde_json::{Map, Value};
    use std::os::unix::fs::symlink;
    use unica_coder::application::UnicaApplication;

    let root = std::env::temp_dir().join(format!(
        "unica-external-init-symlink-{}",
        std::process::id()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join("erf")).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: processors\n",
            "    type: EXTERNAL_DATA_PROCESSORS\n",
            "    path: epf\n",
            "  - name: reports\n",
            "    type: EXTERNAL_REPORTS\n",
            "    path: erf\n",
        ),
    )
    .unwrap();
    symlink(workspace.join("erf"), workspace.join("epf-link")).unwrap();

    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(true));
    args.insert("Name".to_string(), Value::String("Preview".to_string()));
    args.insert(
        "OutputDir".to_string(),
        Value::String("epf-link".to_string()),
    );

    let error = UnicaApplication::new()
        .call_tool("unica.epf.init", &args)
        .unwrap_err();

    assert!(error.contains("must not traverse symlink"), "{error}");
    let _ = std::fs::remove_dir_all(root);
}
