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
    let command_cwd = diagnostic["remediation"]["commands"][0]["cwd"]
        .as_str()
        .expect("remediation cwd");
    assert_eq!(
        Path::new(command_cwd).canonicalize().unwrap(),
        root.canonicalize().unwrap()
    );
    assert_eq!(
        diagnostic["remediation"]["commands"][0]["argv"][4],
        "workspace/src/ConfigDumpInfo.xml"
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn runtime_sidecar_remediation_executes_with_literal_git_pathspecs() {
    let root = temp_root("literal-runtime-sidecar-remediation");
    git(&root, &["init"]);
    create_platform_workspace(&root, ":(glob)*");
    fs::create_dir_all(root.join("safe")).unwrap();
    fs::write(
        root.join(":(glob)*/ConfigDumpInfo.xml"),
        "<ConfigDumpInfo/>\n",
    )
    .unwrap();
    fs::write(root.join("safe/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>\n").unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(
        &root,
        &[
            "--literal-pathspecs",
            "add",
            ".gitignore",
            ".gitattributes",
            "v8project.yaml",
            ":(glob)*/Configuration.xml",
        ],
    );
    git(
        &root,
        &[
            "--literal-pathspecs",
            "add",
            "-f",
            ":(glob)*/ConfigDumpInfo.xml",
            "safe/ConfigDumpInfo.xml",
        ],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    let command = data["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| {
            diagnostic["code"] == "git.runtime_sidecar_tracked"
                && diagnostic["paths"].as_array().is_some_and(|paths| {
                    paths
                        .iter()
                        .any(|path| path == ":(glob)*/ConfigDumpInfo.xml")
                })
        })
        .expect("runtime sidecar diagnostic")["remediation"]["commands"][0]
        .clone();
    let argv = command["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    let output = git_output(&root, &argv);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let staged = git_with_input(&root, &["ls-files", "-z"], b"");
    assert!(!staged
        .split('\0')
        .any(|path| path == ":(glob)*/ConfigDumpInfo.xml"));
    assert!(staged
        .split('\0')
        .any(|path| path == "safe/ConfigDumpInfo.xml"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_fails_each_equal_root_owner_for_shared_generated_paths() {
    let root = temp_root("equal-root-generated-paths");
    git(&root, &["init"]);
    fs::create_dir_all(root.join("src/.build")).unwrap();
    fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>\n").unwrap();
    fs::write(root.join("src/.build/generated.bin"), "generated\n").unwrap();
    fs::write(root.join("src/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: first\n    type: CONFIGURATION\n    path: src\n  - name: second\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
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
    git(
        &root,
        &[
            "add",
            "-f",
            "src/.build/generated.bin",
            "src/ConfigDumpInfo.xml",
        ],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    for check in ["repository.generated_paths", "repository.config_dump_info"] {
        for source_set in [None, Some("first"), Some("second")] {
            assert_repository_check_status(&data, check, source_set, "failed");
        }
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_reports_case_variant_build_path_from_index() {
    let root = temp_root("case-variant-build-path");
    git(&root, &["init"]);
    create_platform_workspace(&root, "src");
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["config", "core.ignorecase", "true"]);
    let oid = git_with_input(&root, &["hash-object", "-w", "--stdin"], b"generated\n");
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{},src/.BUILD/generated.bin", oid.trim()),
        ],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_repository_check_status(&data, "repository.generated_paths", None, "failed");
    assert_repository_check_status(
        &data,
        "repository.generated_paths",
        Some("main"),
        "failed",
    );
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.generated_path_tracked"
            && diagnostic["paths"].as_array().is_some_and(|paths| {
                paths
                    .iter()
                    .any(|path| path == "src/.BUILD/generated.bin")
            })
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_owns_a_filesystem_caseless_unicode_staged_path() {
    let root = temp_root("caseless-unicode-staged-path");
    git(&root, &["init"]);
    create_platform_workspace(&root, "ß");
    let sharp_identity = fs::canonicalize(root.join("ß")).unwrap();
    let Ok(capital_sharp_identity) = fs::canonicalize(root.join("ẞ")) else {
        let _ = fs::remove_dir_all(root);
        return;
    };
    if sharp_identity != capital_sharp_identity {
        let _ = fs::remove_dir_all(root);
        return;
    }
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(&root, &["add", ".gitignore", "v8project.yaml"]);
    let oid = git_with_input(
        &root,
        &["hash-object", "-w", "--stdin"],
        b"<MetaDataObject/>\n",
    );
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{},ẞ/Configuration.xml", oid.trim()),
        ],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_repository_check_status(&data, "repository.attributes", None, "failed");
    assert_repository_check_status(&data, "repository.attributes", Some("main"), "failed");
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|item| {
        item["code"] == "git.text_policy_missing"
            && item["sourceSet"] == "main"
            && item["paths"]
                .as_array()
                .is_some_and(|paths| paths.iter().any(|path| path == "ẞ/Configuration.xml"))
    }));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_does_not_apply_a_host_alias_gitignore_to_another_git_path() {
    let root = temp_root("gitignore-host-alias");
    git(&root, &["init"]);
    fs::create_dir_all(root.join("A/src")).unwrap();
    let upper_identity = fs::canonicalize(root.join("A")).unwrap();
    let Ok(lower_identity) = fs::canonicalize(root.join("a")) else {
        let _ = fs::remove_dir_all(root);
        return;
    };
    if upper_identity != lower_identity {
        let _ = fs::remove_dir_all(root);
        return;
    }
    fs::write(root.join("A/src/Configuration.xml"), "<MetaDataObject/>\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: a/src\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "**/.build/\n").unwrap();
    fs::write(
        root.join("A/.gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(
        &root,
        &["add", ".gitignore", "A/.gitignore", "v8project.yaml"],
    );
    let oid = git_with_input(
        &root,
        &["hash-object", "-w", "--stdin"],
        b"<MetaDataObject/>\n",
    );
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{},a/src/Configuration.xml", oid.trim()),
        ],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_repository_check_status(&data, "repository.ignore", None, "notRun");
    assert_repository_check_status(&data, "repository.ignore", Some("main"), "notRun");
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.inspection_incomplete"
            && diagnostic["evidence"]
                .as_array()
                .is_some_and(|evidence| evidence.iter().any(|item| {
                    item.as_str().is_some_and(|text| text.contains("host path identity"))
                }))
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_uses_staged_external_descriptor_for_repository_resource_policy() {
    let root = temp_root("staged-external-descriptor");
    git(&root, &["init"]);
    fs::create_dir_all(root.join("reports")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: reports\n    type: EXTERNAL_REPORTS\n    path: reports\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(&root, &["add", "v8project.yaml", ".gitignore"]);
    let oid = git_with_input(
        &root,
        &["hash-object", "-w", "--stdin"],
        b"<MetaDataObject><ExternalReport/></MetaDataObject>\n",
    );
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{},reports/Report.xml", oid.trim()),
        ],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_eq!(data["repositoryReady"], false, "{data}");
    assert_repository_check_status(&data, "repository.attributes", None, "failed");
    assert_repository_check_status(
        &data,
        "repository.attributes",
        Some("reports"),
        "failed",
    );
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.text_policy_missing"
            && diagnostic["sourceSet"] == "reports"
            && diagnostic["paths"].as_array().is_some_and(|paths| {
                paths.iter().any(|path| path == "reports/Report.xml")
            })
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_platform_marker_completes_repository_resource_aggregate() {
    let root = temp_root("staged-platform-resource-aggregate");
    git(&root, &["init"]);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
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
        &["add", "v8project.yaml", ".gitignore", ".gitattributes"],
    );
    let marker_oid = git_with_input(
        &root,
        &["hash-object", "-w", "--stdin"],
        b"<MetaDataObject/>\n",
    );
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{},src/Configuration.xml", marker_oid.trim()),
        ],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_eq!(data["ready"], false);
    assert_eq!(data["repositoryReady"], false);
    assert_eq!(data["sourceSets"][0]["sourceFormat"], "unknown");
    for check in [
        "repository.attributes",
        "repository.index_eol",
        "repository.working_eol",
        "repository.lfs",
    ] {
        assert_repository_check_status(&data, check, None, "passed");
        assert_repository_check_status(&data, check, Some("main"), "passed");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_uses_staged_config_dump_descriptor_for_repository_resource_policy() {
    let root = temp_root("staged-config-dump-descriptor");
    git(&root, &["init"]);
    fs::create_dir_all(root.join("reports")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: reports\n    type: EXTERNAL_REPORTS\n    path: reports\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(&root, &["add", "v8project.yaml", ".gitignore"]);
    let oid = git_with_input(
        &root,
        &["hash-object", "-w", "--stdin"],
        b"<MetaDataObject><ExternalReport/></MetaDataObject>\n",
    );
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{},reports/ConfigDumpInfo.xml", oid.trim()),
        ],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_eq!(data["repositoryReady"], false, "{data}");
    assert_repository_check_status(&data, "repository.attributes", None, "failed");
    assert_repository_check_status(
        &data,
        "repository.attributes",
        Some("reports"),
        "failed",
    );
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.text_policy_missing"
            && diagnostic["sourceSet"] == "reports"
            && diagnostic["paths"].as_array().is_some_and(|paths| {
                paths
                    .iter()
                    .any(|path| path == "reports/ConfigDumpInfo.xml")
            })
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_does_not_emit_resource_derivatives_for_inconclusive_config_dump_info() {
    let root = temp_root("inconclusive-config-dump-role");
    git(&root, &["init"]);
    create_platform_workspace(&root, "src");
    fs::write(
        root.join("src/ConfigDumpInfo.xml"),
        "<not-a-platform-descriptor/>\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitattributes"),
        "src/Configuration.xml text eol=lf\n",
    )
    .unwrap();
    git(
        &root,
        &[
            "add",
            "v8project.yaml",
            "src/Configuration.xml",
            ".gitignore",
            ".gitattributes",
        ],
    );
    git(&root, &["add", "-f", "src/ConfigDumpInfo.xml"]);

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.config_dump_info_unclassified"
            && diagnostic["paths"].as_array().is_some_and(|paths| {
                paths
                    .iter()
                    .any(|path| path == "src/ConfigDumpInfo.xml")
            })
    }), "{data}");
    assert!(data["diagnostics"].as_array().unwrap().iter().all(|diagnostic| {
        diagnostic["code"] == "git.config_dump_info_unclassified"
            || !diagnostic["paths"].as_array().is_some_and(|paths| {
                paths
                    .iter()
                    .any(|path| path == "src/ConfigDumpInfo.xml")
            })
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_rejects_cross_kind_staged_config_dump_descriptors() {
    for (case, declared_type, descriptor_child) in [
        (
            "processor-in-report",
            "EXTERNAL_REPORTS",
            "ExternalDataProcessor",
        ),
        (
            "report-in-processor",
            "EXTERNAL_DATA_PROCESSORS",
            "ExternalReport",
        ),
    ] {
        let root = temp_root(case);
        git(&root, &["init"]);
        fs::create_dir_all(root.join("external")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            format!(
                "format: EDT\nsource-set:\n  - name: external\n    type: {declared_type}\n    path: external\n"
            ),
        )
        .unwrap();
        fs::write(
            root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        git(&root, &["add", "v8project.yaml", ".gitignore"]);
        let descriptor = format!("<MetaDataObject><{descriptor_child}/></MetaDataObject>\n");
        let oid = git_with_input(
            &root,
            &["hash-object", "-w", "--stdin"],
            descriptor.as_bytes(),
        );
        git(
            &root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!(
                    "100644,{},external/ConfigDumpInfo.xml",
                    oid.trim()
                ),
            ],
        );

        let result = status(&root);

        assert!(result.ok, "{case}: {:?}", result.errors);
        let data = result.data.unwrap();
        assert_eq!(data["repositoryReady"], false, "{case}: {data}");
        assert_repository_check_status(
            &data,
            "repository.config_dump_info",
            None,
            "failed",
        );
        assert_repository_check_status(
            &data,
            "repository.config_dump_info",
            Some("external"),
            "failed",
        );
        assert_repository_check_status(&data, "repository.attributes", None, "notRun");
        assert_repository_check_status(
            &data,
            "repository.attributes",
            Some("external"),
            "notRun",
        );
        assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
            diagnostic["code"] == "git.config_dump_info_unclassified"
                && diagnostic["sourceSet"] == "external"
                && diagnostic["evidence"].as_array().is_some_and(|evidence| {
                    evidence
                        .iter()
                        .any(|item| item.as_str().is_some_and(|item| item.contains("does not match")))
                })
        }), "{case}: {data}");
        let _ = fs::remove_dir_all(root);
    }
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

#[test]
fn project_health_mixed_platform_and_nested_edt_publish_profile_specific_checks() {
    let root = temp_root("mixed-platform-edt");
    git(&root, &["init"]);
    fs::create_dir_all(root.join("src/edt")).unwrap();
    fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>\n").unwrap();
    fs::write(root.join("src/edt/.project"), "<projectDescription/>\n").unwrap();
    fs::write(root.join("src/edt/Foo.xml"), "<edt/>\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "source-set:\n  - name: designer\n    type: CONFIGURATION\n    path: src\n  - name: edt\n    type: CONFIGURATION\n    path: src/edt\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&root, &["add", "."]);

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    for check in [
        "repository.attributes",
        "repository.index_eol",
        "repository.working_eol",
        "repository.lfs",
    ] {
        assert!(data["checks"].as_array().unwrap().iter().any(|row| {
            row["id"] == check
                && row["sourceSet"] == "designer"
                && row["status"] == "passed"
        }), "{check}/designer: {data}");
        assert!(data["checks"].as_array().unwrap().iter().any(|row| {
            row["id"] == check
                && row["sourceSet"] == "edt"
                && row["status"] == "notApplicable"
        }), "{check}/edt: {data}");
    }
    assert!(!data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.text_policy_missing"
            && diagnostic["paths"].as_array().is_some_and(|paths| {
                paths.iter().any(|path| path == "src/edt/Foo.xml")
            })
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_checks_a_proven_platform_root_when_a_sibling_format_is_unknown() {
    let root = temp_root("platform-with-unknown-sibling");
    git(&root, &["init"]);
    fs::create_dir_all(root.join("good")).unwrap();
    fs::create_dir_all(root.join("unknown")).unwrap();
    fs::write(root.join("good/Configuration.xml"), "<MetaDataObject/>\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "source-set:\n  - name: good\n    type: CONFIGURATION\n    path: good\n  - name: unknown\n    type: CONFIGURATION\n    path: unknown\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "good/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(root.join(".gitattributes"), "good/** text eol=lf\n").unwrap();
    git(&root, &["add", "."]);

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    for check in [
        "repository.attributes",
        "repository.index_eol",
        "repository.working_eol",
        "repository.lfs",
    ] {
        assert!(
            data["checks"].as_array().unwrap().iter().any(|row| {
                row["id"] == check
                    && row["sourceSet"] == "good"
                    && row["status"] == "passed"
            }),
            "{check}/good: {data}"
        );
        assert!(
            data["checks"].as_array().unwrap().iter().any(|row| {
                row["id"] == check
                    && row["sourceSet"] == "unknown"
                    && row["status"] == "notRun"
            }),
            "{check}/unknown: {data}"
        );
    }
    assert!(data["checks"].as_array().unwrap().iter().any(|row| {
        row["id"] == "repository.attributes"
            && row.get("sourceSet").is_none()
            && row["status"] == "notRun"
    }), "aggregate attributes must remain incomplete: {data}");
    assert_repository_check_status(&data, "repository.ignore", None, "failed");
    assert_repository_check_status(&data, "repository.ignore", Some("good"), "passed");
    assert_repository_check_status(&data, "repository.ignore", Some("unknown"), "notRun");
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.ignore_rule_missing"
            && diagnostic.get("sourceSet").is_none()
            && diagnostic["paths"].as_array().is_some_and(|paths| {
                paths
                    .iter()
                    .any(|path| path.as_str().is_some_and(|path| path.contains(".build")))
            })
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_workspace_root_rejection_suppresses_source_derived_git_facts() {
    let root = temp_root("workspace-root-rejected");
    git(&root, &["init"]);
    fs::write(root.join("Configuration.xml"), "<MetaDataObject/>\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "**/.build/\n").unwrap();
    git(
        &root,
        &["add", "v8project.yaml", "Configuration.xml", ".gitignore"],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "source_set.root_is_workspace"
    }), "{data}");
    assert!(!data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["sourceSet"] == "main"
            && matches!(
                diagnostic["code"].as_str(),
                Some(
                    "git.ignore_rule_missing"
                        | "git.ignore_rule_local_only"
                        | "git.generated_path_tracked"
                        | "git.runtime_sidecar_tracked"
                        | "git.config_dump_info_unclassified"
                )
            )
    }), "{data}");
    for check in [
        "repository.ignore",
        "repository.generated_paths",
        "repository.config_dump_info",
        "repository.attributes",
        "repository.index_eol",
        "repository.working_eol",
        "repository.lfs",
    ] {
        assert!(data["checks"].as_array().unwrap().iter().any(|row| {
            row["id"] == check
                && row["sourceSet"] == "main"
                && row["status"] == "notRun"
        }), "{check}/main: {data}");
    }
    for check in [
        "repository.ignore",
        "repository.generated_paths",
        "repository.config_dump_info",
    ] {
        assert_repository_check_status(&data, check, None, "notRun");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_preserves_an_independent_cache_ignore_failure_for_a_rejected_root() {
    let root = temp_root("workspace-root-cache-ignore-missing");
    git(&root, &["init"]);
    fs::write(root.join("Configuration.xml"), "<MetaDataObject/>\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .unwrap();
    git(&root, &["add", "v8project.yaml", "Configuration.xml"]);

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_repository_check_status(&data, "repository.ignore", None, "failed");
    assert_repository_check_status(&data, "repository.ignore", Some("main"), "notRun");
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.ignore_rule_missing"
            && diagnostic.get("sourceSet").is_none()
            && diagnostic["paths"].as_array().is_some_and(|paths| {
                paths
                    .iter()
                    .any(|path| path.as_str().is_some_and(|path| path.contains(".build")))
            })
    }), "{data}");
    assert!(!data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.ignore_rule_missing"
            && diagnostic["sourceSet"] == "main"
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_checks_a_proven_root_when_a_sibling_route_is_unsafe() {
    let root = temp_root("platform-with-unsafe-sibling");
    git(&root, &["init"]);
    fs::create_dir_all(root.join("good")).unwrap();
    fs::write(root.join("good/Configuration.xml"), "<MetaDataObject/>\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: good\n    type: CONFIGURATION\n    path: good\n  - name: unsafe\n    type: CONFIGURATION\n    path: ../outside\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(root.join(".gitattributes"), "good/** text eol=lf\n").unwrap();
    git(&root, &["add", "."]);

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    for check in [
        "repository.ignore",
        "repository.generated_paths",
        "repository.config_dump_info",
        "repository.attributes",
        "repository.index_eol",
        "repository.working_eol",
        "repository.lfs",
    ] {
        assert!(data["checks"].as_array().unwrap().iter().any(|row| {
            row["id"] == check
                && row["sourceSet"] == "good"
                && row["status"] == "passed"
        }), "{check}/good: {data}");
        assert!(data["checks"].as_array().unwrap().iter().any(|row| {
            row["id"] == check
                && row["sourceSet"] == "unsafe"
                && row["status"] == "notRun"
        }), "{check}/unsafe: {data}");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_keeps_resource_policy_independent_between_source_sets() {
    let root = temp_root("independent-resource-roots");
    git(&root, &["init"]);
    for source in ["good", "bad"] {
        fs::create_dir_all(root.join(source)).unwrap();
        fs::write(
            root.join(source).join("Configuration.xml"),
            "<MetaDataObject/>\n",
        )
        .unwrap();
    }
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: good\n    type: CONFIGURATION\n    path: good\n  - name: bad\n    type: CONFIGURATION\n    path: bad\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&root, &["add", "."]);
    let oid = git_with_input(&root, &["hash-object", "-w", "--stdin"], b"target\n");
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{},bad/B.xml", oid.trim()),
        ],
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    for check in [
        "repository.attributes",
        "repository.index_eol",
        "repository.working_eol",
        "repository.lfs",
    ] {
        assert!(data["checks"].as_array().unwrap().iter().any(|row| {
            row["id"] == check
                && row.get("sourceSet").is_none()
                && row["status"] == "notRun"
        }), "{check}/aggregate: {data}");
        assert!(data["checks"].as_array().unwrap().iter().any(|row| {
            row["id"] == check
                && row["sourceSet"] == "good"
                && row["status"] == "passed"
        }), "{check}/good: {data}");
        assert!(data["checks"].as_array().unwrap().iter().any(|row| {
            row["id"] == check
                && row["sourceSet"] == "bad"
                && row["status"] == "notRun"
        }), "{check}/bad: {data}");
    }
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.inspection_incomplete"
            && diagnostic["sourceSet"] == "bad"
            && diagnostic["evidence"].as_array().is_some_and(|evidence| {
                evidence.iter().any(|item| item.as_str().is_some_and(|text| {
                    text.contains("bad/B.xml")
                }))
            })
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn project_health_keeps_working_eol_errors_scoped_to_the_source_set() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp_root("independent-working-eol");
    git(&root, &["init"]);
    for source in ["good", "bad"] {
        fs::create_dir_all(root.join(source)).unwrap();
        fs::write(
            root.join(source).join("Configuration.xml"),
            "<MetaDataObject/>\n",
        )
        .unwrap();
        fs::write(root.join(source).join("Extra.xml"), "<A/>\n").unwrap();
    }
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: good\n    type: CONFIGURATION\n    path: good\n  - name: bad\n    type: CONFIGURATION\n    path: bad\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&root, &["add", "."]);
    fs::write(
        root.join("bad/Configuration.xml"),
        "<MetaDataObject>\n<A/>\r\n</MetaDataObject>\n",
    )
    .unwrap();
    fs::set_permissions(root.join("bad/Extra.xml"), fs::Permissions::from_mode(0o0)).unwrap();
    if fs::File::open(root.join("bad/Extra.xml")).is_ok() {
        fs::set_permissions(
            root.join("bad/Extra.xml"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let _ = fs::remove_dir_all(root);
        return;
    }

    let result = status(&root);

    fs::set_permissions(
        root.join("bad/Extra.xml"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_repository_check_status(&data, "repository.working_eol", None, "notRun");
    assert_repository_check_status(&data, "repository.working_eol", Some("good"), "passed");
    assert_repository_check_status(&data, "repository.working_eol", Some("bad"), "notRun");
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.inspection_incomplete"
            && diagnostic["sourceSet"] == "bad"
            && diagnostic["evidence"].as_array().is_some_and(|evidence| {
                evidence.iter().any(|item| item.as_str().is_some_and(|text| {
                    text.contains("bad/Extra.xml")
                }))
            })
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_keeps_lfs_errors_scoped_to_the_source_set() {
    let root = temp_root("independent-lfs");
    git(&root, &["init"]);
    for source in ["good", "bad"] {
        fs::create_dir_all(root.join(source)).unwrap();
        fs::write(
            root.join(source).join("Configuration.xml"),
            "<MetaDataObject/>\n",
        )
        .unwrap();
        fs::write(root.join(source).join("Picture.bin"), b"binary").unwrap();
    }
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: good\n    type: CONFIGURATION\n    path: good\n  - name: bad\n    type: CONFIGURATION\n    path: bad\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(
        root.join(".gitattributes"),
        "*.xml text eol=lf\n*.bin -text\n",
    )
    .unwrap();
    fs::File::create(root.join("bad/A-Large.bin"))
        .unwrap()
        .set_len(10 * 1024 * 1024)
        .unwrap();
    git(&root, &["add", "."]);
    fs::remove_file(root.join("bad/Picture.bin")).unwrap();
    fs::create_dir(root.join("bad/Picture.bin")).unwrap();

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_repository_check_status(&data, "repository.lfs", None, "notRun");
    assert_repository_check_status(&data, "repository.lfs", Some("good"), "passed");
    assert_repository_check_status(&data, "repository.lfs", Some("bad"), "notRun");
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.inspection_incomplete"
            && diagnostic["sourceSet"] == "bad"
            && diagnostic["evidence"].as_array().is_some_and(|evidence| {
                evidence.iter().any(|item| item.as_str().is_some_and(|text| {
                    text.contains("bad/Picture.bin")
                }))
            })
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_reports_index_eol_even_when_text_policy_is_missing() {
    let root = temp_root("missing-attributes-crlf-index");
    git(&root, &["init"]);
    create_platform_workspace(&root, "src");
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    fs::write(
        root.join("src/Configuration.xml"),
        "<MetaDataObject/>\r\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    let crlf_blob_output = git_with_input(
        &root,
        &["hash-object", "-w", "--stdin"],
        b"<MetaDataObject/>\r\n",
    );
    let crlf_blob = crlf_blob_output
        .strip_suffix('\n')
        .expect("git hash-object terminates the object id with a newline");
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{crlf_blob},src/Configuration.xml"),
        ],
    );
    assert_eq!(
        git_output(&root, &["show", ":src/Configuration.xml"]).stdout,
        b"<MetaDataObject/>\r\n"
    );

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_repository_check_status(&data, "repository.attributes", Some("main"), "failed");
    assert_repository_check_status(&data, "repository.index_eol", Some("main"), "failed");
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.index_eol_not_lf"
    }), "{data}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_health_reports_working_eol_even_when_text_policy_is_local_only() {
    let root = temp_root("local-only-attributes-mixed-working-eol");
    git(&root, &["init"]);
    create_platform_workspace(&root, "src");
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    fs::write(root.join(".git/info/attributes"), "*.xml text eol=lf\n").unwrap();
    fs::write(
        root.join("src/Configuration.xml"),
        "<MetaDataObject>\n<A/>\r\n</MetaDataObject>\n",
    )
    .unwrap();

    let result = status(&root);

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert_repository_check_status(&data, "repository.attributes", Some("main"), "failed");
    assert_repository_check_status(&data, "repository.working_eol", Some("main"), "failed");
    assert!(data["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["code"] == "git.mixed_eol"
    }), "{data}");
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

#[test]
fn project_health_bounds_equal_root_resource_ownership_composition() {
    let root = temp_root("equal-root-resource-scale");
    git(&root, &["init"]);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>\n").unwrap();
    for index in 0..63 {
        fs::write(root.join(format!("src/Module{index}.bsl")), "Процедура P()\nКонецПроцедуры\n")
            .unwrap();
    }
    let source_sets = (0..1024)
        .map(|index| {
            format!(
                "  - name: owner-{index:04}\n    type: CONFIGURATION\n    path: src\n"
            )
        })
        .collect::<String>();
    fs::write(
        root.join("v8project.yaml"),
        format!("format: DESIGNER\nsource-set:\n{source_sets}"),
    )
    .unwrap();
    fs::write(
        root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(&root, &["add", "v8project.yaml", ".gitignore", "src"]);

    let result = status(&root);

    assert!(result.ok, "errors={:?}", result.errors);
    let data = result.data.unwrap();
    assert_repository_check_status(&data, "repository.attributes", None, "notRun");
    assert_repository_check_status(
        &data,
        "repository.attributes",
        Some("owner-0000"),
        "notRun",
    );
    assert_repository_check_status(
        &data,
        "repository.attributes",
        Some("owner-1023"),
        "notRun",
    );
    let owner_diagnostic = data["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| {
            diagnostic["code"] == "git.inspection_incomplete"
                && diagnostic["sourceSet"] == "owner-0000"
        })
        .unwrap();
    assert_eq!(owner_diagnostic["count"], 64, "{data}");
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

fn assert_repository_check_status(
    data: &Value,
    check: &str,
    source_set: Option<&str>,
    expected: &str,
) {
    assert!(data["checks"].as_array().unwrap().iter().any(|row| {
        row["id"] == check
            && row.get("sourceSet").and_then(Value::as_str) == source_set
            && row["status"] == expected
    }), "{check}/{source_set:?} expected {expected}: {data}");
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
