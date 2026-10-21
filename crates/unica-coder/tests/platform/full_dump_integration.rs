//! Cross-owner v8 execution, retained publication and Designer validation oracles.
use roxmltree::Document;
use std::path::Path;
use unica_adapter_v8_runner::full_dump::FullDumpSelection;
use unica_adapter_v8_runner::full_dump::{
    attest_selected_local, dump_failure, finalize_private_outcome, merge_yaml_values,
    prepare_legacy, prepare_publication, with_test_platform, write_projection, FullDumpInvocation,
    PlatformResolver, PlatformUtility, SystemPlatformResolver, VerifiedFullDumpAdapter,
    VerifiedPlatform,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::roots::{
    platform_xml_publication_policy as staged_root_version_policy,
    PlatformXmlPublicationPolicy as StagedRootVersionPolicy,
    PLATFORM_XML_ROOTS as STAGED_ROOT_REGISTRY,
};
use unica_workspace::full_dump::test_support::*;
use unica_workspace::full_dump::{
    FullDumpPhase, PlatformAttestation, RunnerProjectLockFailure, RunnerProjectStore,
    VerifiedFullDumpPublication,
};
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::read::{FileIdentity, ReadFile};
const TARGET_EXPORT_FORMAT: &str =
    unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE.export_format;
const EFFECTIVE_CONFIG_NAME: &str = "v8project.yaml";
const LOCAL_CONFIG_NAME: &str = "v8project.local.yaml";
fn validate_staged_dump(root: &Path, kind: SourceSetKind) -> Result<TreeSnapshot, String> {
    inspect_and_validate_stage(
        root,
        kind,
        crate::infrastructure::runtime_composition::full_dump_semantics().as_ref(),
    )
}
trait FixtureFileObservation {
    fn fixture_identity(&self) -> std::io::Result<FileIdentity>;
    fn fixture_owner_only(&self) -> std::io::Result<()>;
}
impl FixtureFileObservation for std::fs::File {
    fn fixture_identity(&self) -> std::io::Result<FileIdentity> {
        fixture_file_identity(self)
    }
    fn fixture_owner_only(&self) -> std::io::Result<()> {
        fixture_verify_private_owner(self)
    }
}
impl FixtureFileObservation for ReadFile {
    fn fixture_identity(&self) -> std::io::Result<FileIdentity> {
        self.identity()
    }
    fn fixture_owner_only(&self) -> std::io::Result<()> {
        self.verify_owner_only()
    }
}
fn file_identity(file: &impl FixtureFileObservation) -> std::io::Result<FileIdentity> {
    file.fixture_identity()
}
fn verify_private_owner(file: &impl FixtureFileObservation) -> std::io::Result<()> {
    file.fixture_owner_only()
}
use unica_workspace::read::open_read_directory_nofollow as open_directory_nofollow;
trait FixtureRunnerProjectStore {
    fn write_config(&mut self, name: &str, value: &serde_yaml::Value) -> Result<(), String>;
    fn remove_config(&mut self, name: &str) -> Result<(), String>;
}
impl FixtureRunnerProjectStore for RunnerProjectStore {
    fn write_config(&mut self, name: &str, value: &serde_yaml::Value) -> Result<(), String> {
        self.test_write_config_bytes(
            name,
            serde_yaml::to_string(value)
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )
    }
    fn remove_config(&mut self, name: &str) -> Result<(), String> {
        self.test_remove_config(name)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::with_effective_config_create_hook;
    use super::write_projection;
    use super::FixtureRunnerProjectStore;
    #[cfg_attr(windows, allow(unused_imports))]
    use super::{
        staged_root_version_policy, validate_staged_dump, with_private_create_hook,
        with_publication_failpoint, with_publication_hook, with_secure_read_hook,
        with_target_parent_capture_hook, with_targeted_read_hooks, with_tree_open_hook, Document,
        FullDumpInvocation, PlatformResolver, PlatformUtility, PublicationCheckpoint,
        StagedRootVersionPolicy, SystemPlatformResolver, TreeSnapshot, VerifiedFullDumpAdapter,
        VerifiedPlatform, STAGED_ROOT_REGISTRY, TARGET_EXPORT_FORMAT,
    };
    use serde_json::{Map, Value};
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner};
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_source_contract::SourceSetKind;
    use unica_work_control::cancellation::CancellationToken;

    struct FixedPlatform {
        executable: PathBuf,
        version: String,
    }

    impl PlatformResolver for FixedPlatform {
        fn resolve(
            &self,
            _effective_config: &serde_yaml::Value,
            _config_dir: &Path,
            _utility: PlatformUtility,
            _runner: &dyn ProcessRunner,
            _cancellation: &CancellationToken,
        ) -> Result<VerifiedPlatform, String> {
            Ok(VerifiedPlatform {
                executable: self.executable.clone(),
                exact_version: self.version.clone(),
                attestation: super::PlatformAttestation::capture_test_fixture(&self.executable)?,
            })
        }
    }

    struct FailingPlatform;

    impl PlatformResolver for FailingPlatform {
        fn resolve(
            &self,
            _effective_config: &serde_yaml::Value,
            _config_dir: &Path,
            _utility: PlatformUtility,
            _runner: &dyn ProcessRunner,
            _cancellation: &CancellationToken,
        ) -> Result<VerifiedPlatform, String> {
            Err("platform 8.3.26 does not match required 8.3.27".to_string())
        }
    }

    enum RunnerMutation {
        None,
        Config(PathBuf),
        Target(PathBuf),
        PlatformFile(PathBuf),
        PlatformInstall(PathBuf),
        #[cfg(unix)]
        RestrictPrivateRoot,
        #[cfg(unix)]
        Symlink,
        #[cfg(unix)]
        SwapPrivateRoot(std::rc::Rc<std::cell::Cell<Option<super::FileIdentity>>>),
    }

    struct DumpRunner {
        owner_xml: String,
        mutation: RunnerMutation,
        calls: AtomicUsize,
    }

    impl DumpRunner {
        fn valid() -> Self {
            Self {
                owner_xml: valid_configuration_owner(),
                mutation: RunnerMutation::None,
                calls: AtomicUsize::new(0),
            }
        }
    }

    struct SwapRestoreProbeRunner {
        install: PathBuf,
        calls: AtomicUsize,
    }

    impl ProcessRunner for SwapRestoreProbeRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let displaced = self.install.with_extension("swap-restore-displaced");
            std::fs::rename(&self.install, &displaced).unwrap();
            std::fs::create_dir(&self.install).unwrap();
            std::fs::remove_dir(&self.install).unwrap();
            std::fs::rename(displaced, &self.install).unwrap();
            Ok(ProcessOutput {
                status_success: true,
                status: "0".to_string(),
                stdout: "8.3.27.2074".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            })
        }
    }

    impl ProcessRunner for DumpRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let config_index = command
                .args
                .iter()
                .position(|arg| arg == "--config")
                .expect("effective config flag");
            let config_path = PathBuf::from(&command.args[config_index + 1]);
            let config: serde_yaml::Value =
                serde_yaml::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
            assert!(
                config.get("basePath").is_none(),
                "pinned v8-runner rejects the removed basePath key"
            );
            assert_eq!(config["format"].as_str(), Some("DESIGNER"));
            assert!(Path::new(config["workPath"].as_str().unwrap()).is_absolute());
            let connection = config["infobase"]["connection"].as_str().unwrap();
            if connection.starts_with("/F ") {
                assert!(
                    Path::new(connection.trim_start_matches("/F ")).is_absolute(),
                    "{connection}"
                );
            } else {
                assert!(
                    connection.to_ascii_lowercase().starts_with("file="),
                    "{connection}"
                );
            }
            assert_eq!(
                config["tools"]["platform"]["version"].as_str(),
                Some("8.3.27.2074")
            );
            assert!(config["tools"]["platform"]["path"]
                .as_str()
                .unwrap()
                .contains("8.3.27.2074"));
            let source_sets = config["source-set"].as_sequence().unwrap();
            let target = PathBuf::from(source_sets[0]["path"].as_str().unwrap());
            assert!(
                target
                    .file_name()
                    .is_some_and(|name| name == "staged-source"),
                "{}",
                target.display()
            );
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("Configuration.xml"), self.owner_xml.as_bytes()).unwrap();
            std::fs::write(
                target.join("Catalogs.xml"),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{TARGET_EXPORT_FORMAT}"><Catalog/></MetaDataObject>"#
                ),
            )
            .unwrap();
            match &self.mutation {
                RunnerMutation::None => {}
                RunnerMutation::Config(path) => {
                    std::fs::write(path, b"changed during child execution").unwrap();
                }
                RunnerMutation::Target(path) => {
                    std::fs::write(path.join("concurrent.txt"), b"concurrent").unwrap();
                }
                RunnerMutation::PlatformFile(path) => {
                    std::fs::write(path, b"replaced platform executable").unwrap();
                }
                RunnerMutation::PlatformInstall(path) => {
                    let displaced = path.with_extension("displaced");
                    std::fs::rename(path, &displaced).unwrap();
                    std::fs::create_dir_all(path).unwrap();
                    make_platform_executable(&path.join("1cv8"));
                    make_platform_executable(&path.join("ibcmd"));
                }
                #[cfg(unix)]
                RunnerMutation::RestrictPrivateRoot => {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        config_path.parent().unwrap(),
                        std::fs::Permissions::from_mode(0o500),
                    )
                    .unwrap();
                }
                #[cfg(unix)]
                RunnerMutation::Symlink => {
                    std::os::unix::fs::symlink(
                        target.join("Configuration.xml"),
                        target.join("unsafe-link.xml"),
                    )
                    .unwrap();
                }
                #[cfg(unix)]
                RunnerMutation::SwapPrivateRoot(original_identity) => {
                    use std::os::unix::fs::PermissionsExt;

                    let private_root = config_path
                        .parent()
                        .and_then(Path::parent)
                        .expect("effective config is under the private root");
                    original_identity.set(Some(
                        super::file_identity(
                            &super::open_directory_nofollow(private_root).unwrap(),
                        )
                        .unwrap(),
                    ));
                    let displaced_name = format!(
                        "{}-displaced",
                        private_root.file_name().unwrap().to_string_lossy()
                    );
                    let displaced = private_root.with_file_name(displaced_name);
                    std::fs::rename(private_root, &displaced).unwrap();
                    std::fs::create_dir(private_root).unwrap();
                    std::fs::set_permissions(private_root, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    for child in ["execution", "recovery"] {
                        let path = private_root.join(child);
                        std::fs::create_dir(&path).unwrap();
                        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                            .unwrap();
                    }
                    std::fs::write(
                        private_root.join("execution/foreign.txt"),
                        b"foreign root sentinel",
                    )
                    .unwrap();
                }
            }
            Ok(ProcessOutput {
                status_success: true,
                status: "0".to_string(),
                stdout: "ok".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            })
        }
    }

    fn valid_configuration_owner() -> String {
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties/></Configuration></MetaDataObject>"#.to_string()
    }

    fn seed_valid_target(target: &Path) {
        std::fs::create_dir_all(target).unwrap();
        std::fs::write(
            target.join("Configuration.xml"),
            valid_configuration_owner(),
        )
        .unwrap();
        std::fs::write(target.join("before.txt"), b"before").unwrap();
    }

    fn workspace(name: &str) -> (PathBuf, WorkspaceContext, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-verified-full-dump-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let config = workspace.join("v8project.yaml");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace,
            cache_root: root.join("cache"),
            workspace_epoch: 0,
        };
        (root, context, config)
    }

    fn fixed_platform(root: &Path) -> FixedPlatform {
        let install = root.join("8.3.27.2074");
        let executable = install.join(PlatformUtility::Designer.executable_name());
        make_platform_executable(&executable);
        make_platform_executable(&install.join(PlatformUtility::Ibcmd.executable_name()));
        FixedPlatform {
            executable,
            version: "8.3.27.2074".to_string(),
        }
    }

    fn make_platform_executable(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"#!/bin/sh\nprintf '8.3.27.2074\\n'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
    }

    fn invoke_with_args(
        runner: &DumpRunner,
        platform: &dyn PlatformResolver,
        invocation: FullDumpInvocation,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
    ) -> unica_execution_contract::adapter_outcome::AdapterOutcome {
        VerifiedFullDumpAdapter::with_dependencies(
            runner,
            platform,
            PathBuf::from("fake-v8-runner"),
            crate::infrastructure::runtime_composition::runtime_environment(),
            crate::infrastructure::runtime_composition::full_dump_semantics(),
        )
        .invoke(
            match invocation {
                FullDumpInvocation::BuildDump => "unica.build.dump",
                FullDumpInvocation::RuntimeExecute => "unica.runtime.execute",
            },
            invocation,
            args,
            context,
            &CancellationToken::new(),
        )
        .unwrap()
    }

    fn args() -> Map<String, Value> {
        Map::from_iter([
            ("dryRun".to_string(), Value::Bool(false)),
            ("mode".to_string(), Value::String("full".to_string())),
        ])
    }

    fn invoke(
        runner: &DumpRunner,
        platform: &dyn PlatformResolver,
        invocation: FullDumpInvocation,
        context: &WorkspaceContext,
    ) -> unica_execution_contract::adapter_outcome::AdapterOutcome {
        invoke_with_args(runner, platform, invocation, context, &args())
    }

    fn assert_retained_directory_artifact(
        result: &unica_execution_contract::adapter_outcome::AdapterOutcome,
        retained: &Path,
    ) {
        let expected = std::fs::canonicalize(retained).unwrap();
        let expected_identity =
            super::file_identity(&super::open_directory_nofollow(&expected).unwrap()).unwrap();
        assert!(
            result.artifacts.iter().any(|artifact| {
                std::fs::canonicalize(artifact).is_ok_and(|actual| {
                    actual == expected
                        && super::open_directory_nofollow(&actual).is_ok_and(|directory| {
                            super::file_identity(&directory)
                                .is_ok_and(|identity| identity == expected_identity)
                        })
                })
            }),
            "a recovery artifact must name the retained directory identity"
        );
    }

    #[test]
    fn effective_config_preparation_failure_reports_retained_foreign_data() {
        let (root, context, _) = workspace("effective-config-prepare-retention");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let retained = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_retained = std::rc::Rc::clone(&retained);

        let result = super::with_effective_config_prepare_hook(
            move |path| {
                std::fs::create_dir(path).unwrap();
                std::fs::write(path.join("foreign.txt"), b"foreign sentinel").unwrap();
                let private = path.parent().unwrap().parent().unwrap().to_path_buf();
                let snapshot = super::capture_tree_target_nofollow(&private).unwrap();
                *hook_retained.borrow_mut() = Some((private, snapshot));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok);
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(result.changes.is_empty());
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        let retained = retained.borrow();
        let (private, snapshot) = retained.as_ref().expect("preparation hook must run");
        assert_eq!(
            super::capture_tree_target_nofollow(private).unwrap(),
            *snapshot,
            "preparation cleanup must preserve all retained identities and bytes"
        );
        assert_eq!(
            std::fs::read(
                private
                    .join("execution")
                    .join(super::EFFECTIVE_CONFIG_NAME)
                    .join("foreign.txt")
            )
            .unwrap(),
            b"foreign sentinel"
        );
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("failed to securely create private effective config")));
        assert!(result
            .summary
            .contains("failed before verified dump publication"));
        let expected = std::fs::canonicalize(private).unwrap();
        let has_recovery_artifact = result
            .artifacts
            .iter()
            .any(|artifact| std::fs::canonicalize(artifact).is_ok_and(|path| path == expected));
        let reports_discarded = result
            .warnings
            .iter()
            .any(|warning| warning.contains("the private staged dump was discarded"));
        assert!(
            has_recovery_artifact && !reports_discarded,
            "retained preparation data needs recovery metadata: artifact={has_recovery_artifact}, discarded={reports_discarded}"
        );
        assert_retained_directory_artifact(&result, private);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_applied_full_dump_reaches_runner_and_commits_validated_tree() {
        let (root, context, _) = workspace("windows-applied-full-dump");
        let target = context.cwd.join("src");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
        assert!(target.join("Configuration.xml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_applied_full_dump_rejects_effective_config_acl_mutation_before_runner_and_cleans_created_identity(
    ) {
        use std::os::windows::ffi::OsStrExt;
        use std::ptr;
        use windows_sys::Win32::Security::Authorization::{SetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows_sys::Win32::Security::{
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        };

        let (root, context, _) = workspace("windows-effective-config-acl-mutation");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let displaced = context.cwd.join("created-effective-config.yaml");
        let hook_displaced = displaced.clone();

        let result = with_effective_config_create_hook(
            move |path| {
                std::fs::rename(path, &hook_displaced).unwrap();
                let mut path = hook_displaced.as_os_str().encode_wide().collect::<Vec<_>>();
                path.push(0);
                // SAFETY: path is NUL-terminated and names the newly created file. A null DACL is
                // intentional test corruption that the production verifier must reject.
                let status = unsafe {
                    SetNamedSecurityInfoW(
                        path.as_ptr(),
                        SE_FILE_OBJECT,
                        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null(),
                        ptr::null(),
                    )
                };
                assert_eq!(
                    status,
                    0,
                    "{}",
                    std::io::Error::from_raw_os_error(status as i32)
                );
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(
            !displaced.exists(),
            "the originally created identity escaped cleanup: {}",
            displaced.display()
        );
        assert!(std::fs::read_dir(&context.cwd).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".unica-dump-guard-")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_applied_full_dump_rejects_effective_config_name_swap_before_runner_and_preserves_replacement_identity(
    ) {
        use std::cell::Cell;

        let (root, context, _) = workspace("windows-effective-config-name-swap");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let displaced = context.cwd.join("created-effective-config.yaml");
        let replacement = context.cwd.join("replacement-effective-config.yaml");
        let target = context.cwd.join("src");
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        std::fs::write(&replacement, b"replacement sentinel").unwrap();
        let expected_replacement_identity =
            super::file_identity(&std::fs::File::open(&replacement).unwrap()).unwrap();
        let created_identity = std::rc::Rc::new(Cell::new(None));
        let retained = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_retained = std::rc::Rc::clone(&retained);
        let hook_created_identity = std::rc::Rc::clone(&created_identity);
        let hook_displaced = displaced.clone();
        let hook_replacement = replacement.clone();

        let result = with_effective_config_create_hook(
            move |path| {
                hook_created_identity.set(Some(
                    super::file_identity(&std::fs::File::open(path).unwrap()).unwrap(),
                ));
                std::fs::rename(path, &hook_displaced).unwrap();
                std::fs::hard_link(&hook_replacement, path).unwrap();
                let execution = path.parent().unwrap();
                let private = execution.parent().unwrap();
                let directories = [
                    private.to_path_buf(),
                    execution.to_path_buf(),
                    private.join("recovery"),
                ]
                .into_iter()
                .map(|directory| {
                    let identity = super::file_identity(
                        &super::open_directory_nofollow(
                            &std::fs::canonicalize(&directory).unwrap(),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                    (directory, identity)
                })
                .collect::<Vec<_>>();
                *hook_retained.borrow_mut() = Some((path.to_path_buf(), directories));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok);
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(
            created_identity.get().is_some(),
            "the post-create hook did not run"
        );
        assert!(
            !displaced.exists(),
            "the originally created identity escaped cleanup: {}",
            displaced.display()
        );
        assert_eq!(
            super::file_identity(&std::fs::File::open(&replacement).unwrap(),).unwrap(),
            expected_replacement_identity,
            "cleanup followed the replaced effective-config name"
        );
        assert_eq!(
            std::fs::read(&replacement).unwrap(),
            b"replacement sentinel"
        );
        assert!(result.changes.is_empty());
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        let retained = retained.borrow();
        let (inside, directories) = retained.as_ref().expect("post-create hook must run");
        assert_eq!(
            super::file_identity(&std::fs::File::open(inside).unwrap()).unwrap(),
            expected_replacement_identity,
            "the substituted private name must still refer to the foreign file"
        );
        assert_ne!(Some(expected_replacement_identity), created_identity.get());
        assert_eq!(std::fs::read(inside).unwrap(), b"replacement sentinel");
        for (directory, expected_identity) in directories {
            assert_eq!(
                super::file_identity(
                    &super::open_directory_nofollow(&std::fs::canonicalize(directory).unwrap())
                        .unwrap(),
                )
                .unwrap(),
                *expected_identity,
                "cleanup must retain every original private directory identity"
            );
        }
        let private = &directories[0].0;
        let retained_roots = std::fs::read_dir(&context.cwd)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .map(|path| std::fs::canonicalize(path).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            retained_roots,
            vec![std::fs::canonicalize(private).unwrap()]
        );
        assert_retained_directory_artifact(&result, private);
        assert!(result.errors.iter().any(|error| error
            .contains("private effective config identity changed before verified reopen")));
        assert!(result
            .summary
            .contains("failed before verified dump publication"));
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("Data retained for recovery")));
        assert!(!result
            .warnings
            .iter()
            .any(|warning| warning.contains("the private staged dump was discarded")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selected_source_set_target_must_be_relative_and_physically_contained_in_workspace() {
        for case in [
            "absolute",
            "parent-escape",
            "symlink-parent",
            "target-symlink",
        ] {
            let (root, context, config) = workspace(case);
            let outside = root.join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            let configured_path = match case {
                "absolute" => outside.join("absolute-src").display().to_string(),
                "parent-escape" => "../../outside/parent-src".to_string(),
                "symlink-parent" => {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&outside, context.cwd.join("linked")).unwrap();
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_dir(&outside, context.cwd.join("linked"))
                        .unwrap();
                    #[cfg(not(any(unix, windows)))]
                    std::fs::create_dir_all(context.cwd.join("linked")).unwrap();
                    "linked/symlink-src".to_string()
                }
                "target-symlink" => {
                    let real_target = context.cwd.join("real-target");
                    std::fs::create_dir_all(&real_target).unwrap();
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&real_target, context.cwd.join("src")).unwrap();
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_dir(&real_target, context.cwd.join("src"))
                        .unwrap();
                    #[cfg(not(any(unix, windows)))]
                    std::fs::create_dir_all(context.cwd.join("src")).unwrap();
                    "src".to_string()
                }
                _ => unreachable!(),
            };
            std::fs::write(
                &config,
                format!(
                    "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: '{configured_path}'\n"
                ),
            )
            .unwrap();
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert_eq!(runner.calls.load(Ordering::SeqCst), 0, "{case}");
            assert!(
                result.errors.join("\n").contains("workspace")
                    || result.errors.join("\n").contains("relative")
                    || result.errors.join("\n").contains("symbolic link")
                    || result.errors.join("\n").contains("real directory"),
                "{case}: {result:?}"
            );
            assert!(!outside.join("absolute-src/Configuration.xml").exists());
            assert!(!outside.join("parent-src/Configuration.xml").exists());
            assert!(!outside.join("symlink-src/Configuration.xml").exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn target_parent_symlink_swap_after_text_normalization_cannot_escape_workspace_anchor() {
        let (root, context, config) = workspace("target-parent-normalization-swap");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: inside/src\n",
        )
        .unwrap();
        let inside = context.cwd.join("inside");
        let displaced = context.cwd.join("inside-displaced");
        let outside = root.join("outside");
        std::fs::create_dir(&inside).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let hook_inside = inside.clone();
        let hook_displaced = displaced.clone();
        let hook_outside = outside.clone();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = with_target_parent_capture_hook(
            move |_normalized_parent| {
                std::fs::rename(&hook_inside, &hook_displaced).unwrap();
                std::os::unix::fs::symlink(&hook_outside, &hook_inside).unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(!outside.join("src").exists(), "{result:?}");
        assert!(
            std::fs::read_dir(&outside).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".unica-dump-guard-")),
            "{result:?}"
        );
        assert!(
            result.errors.join("\n").contains("workspace")
                || result.errors.join("\n").contains("link"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn physical_edt_or_mixed_target_cannot_be_replaced_by_designer_yaml_claim() {
        for case in ["edt", "mixed"] {
            let (root, context, _) = workspace(case);
            let target = context.cwd.join("src");
            std::fs::create_dir_all(target.join("Configuration")).unwrap();
            std::fs::write(target.join(".project"), b"<projectDescription/>").unwrap();
            std::fs::write(
                target.join("Configuration/Configuration.mdo"),
                b"<mdclass:Configuration/>",
            )
            .unwrap();
            if case == "mixed" {
                std::fs::write(
                    target.join("Configuration.xml"),
                    valid_configuration_owner(),
                )
                .unwrap();
            }
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert_eq!(runner.calls.load(Ordering::SeqCst), 0, "{case}");
            assert!(target.join(".project").is_file(), "{case}");
            assert!(
                result.errors.join("\n").contains("EDT")
                    || result.errors.join("\n").contains("mixed")
                    || result.errors.join("\n").contains("invalid"),
                "{case}: {result:?}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn existing_legacy_designer_target_requires_explicit_user_migration() {
        let (root, context, _) = workspace("legacy-designer-target");
        let target = context.cwd.join("src");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            target.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><Properties/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(
            result
                .errors
                .join("\n")
                .contains("not be migrated automatically")
                && result.errors.join("\n").contains("8.3.27"),
            "{result:?}"
        );
        assert!(std::fs::read_to_string(target.join("Configuration.xml"))
            .unwrap()
            .contains("2.19"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_synchronous_routes_publish_only_validated_private_stage() {
        for invocation in [
            FullDumpInvocation::BuildDump,
            FullDumpInvocation::RuntimeExecute,
        ] {
            let (root, context, _) = workspace(match invocation {
                FullDumpInvocation::BuildDump => "build-success",
                FullDumpInvocation::RuntimeExecute => "runtime-success",
            });
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();

            let result = invoke(&runner, &platform, invocation, &context);

            assert!(result.ok, "{invocation:?}: {result:?}");
            assert!(context.cwd.join("src/Configuration.xml").is_file());
            assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
            assert!(
                std::fs::read_dir(&context.cwd).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".unica-dump-guard-")),
                "private stage debris remained"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[derive(Clone, Copy)]
    enum IncompleteRunnerFinish {
        RunError,
        TimedOut,
        Cancelled,
        FailedExit,
        Panic,
        SpawnFailure,
    }

    struct WaitingDumpWriter {
        release: Option<std::sync::mpsc::Sender<()>>,
        writer: Option<std::thread::JoinHandle<std::io::Result<()>>>,
    }

    impl WaitingDumpWriter {
        fn finish(&mut self) -> std::io::Result<()> {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            self.writer
                .take()
                .expect("writer is retained")
                .join()
                .unwrap()
        }
    }

    impl Drop for WaitingDumpWriter {
        fn drop(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            if let Some(writer) = self.writer.take() {
                let _ = writer.join();
            }
        }
    }

    struct IncompleteDumpRunner {
        finish: IncompleteRunnerFinish,
        create_stage: bool,
        private_paths: std::cell::RefCell<Option<(PathBuf, PathBuf, PathBuf)>>,
        pending_writer: std::cell::RefCell<Option<WaitingDumpWriter>>,
        spawn_output_snapshot: std::cell::RefCell<Option<TreeSnapshot>>,
    }

    impl IncompleteDumpRunner {
        fn new(finish: IncompleteRunnerFinish, create_stage: bool) -> Self {
            Self {
                finish,
                create_stage,
                private_paths: std::cell::RefCell::new(None),
                pending_writer: std::cell::RefCell::new(None),
                spawn_output_snapshot: std::cell::RefCell::new(None),
            }
        }

        fn remember_paths(&self, command: &ProcessCommand) -> PathBuf {
            let config_index = command
                .args
                .iter()
                .position(|arg| arg == "--config")
                .unwrap();
            let config_path = PathBuf::from(&command.args[config_index + 1]);
            let config: serde_yaml::Value =
                serde_yaml::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
            let stage = PathBuf::from(config["source-set"][0]["path"].as_str().unwrap());
            let private_root = config_path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf();
            self.private_paths
                .replace(Some((private_root, config_path, stage.clone())));
            stage
        }
    }

    impl ProcessRunner for IncompleteDumpRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let stage = self.remember_paths(command);
            if matches!(self.finish, IncompleteRunnerFinish::SpawnFailure) {
                return Err("the operating system refused to launch the runner".into());
            }
            if self.create_stage {
                std::fs::create_dir(&stage).unwrap();
            }
            let (release, wait) = std::sync::mpsc::channel();
            let writer = std::thread::spawn(move || {
                wait.recv().unwrap();
                if !stage.exists() {
                    // A writer may create its own output, but cannot recreate
                    // the enclosing attempt after premature cleanup.
                    std::fs::create_dir(&stage)?;
                }
                std::fs::write(stage.join("late-writer.txt"), b"data after runner returned")
            });
            self.pending_writer.replace(Some(WaitingDumpWriter {
                release: Some(release),
                writer: Some(writer),
            }));
            match self.finish {
                IncompleteRunnerFinish::RunError => {
                    Err("runner wait failed before writer completion was confirmed".into())
                }
                IncompleteRunnerFinish::Panic => panic!("injected runner unwind"),
                _ => Ok(ProcessOutput {
                    status_success: false,
                    status: "1".into(),
                    stdout: String::new(),
                    stderr: "runner completion was not confirmed".into(),
                    timed_out: matches!(self.finish, IncompleteRunnerFinish::TimedOut),
                    cancelled: matches!(self.finish, IncompleteRunnerFinish::Cancelled),
                    stdout_truncated: false,
                    stderr_truncated: false,
                    stdout_had_invalid_utf8: false,
                    stderr_had_invalid_utf8: false,
                }),
            }
        }

        fn run_staged(
            &self,
            command: &ProcessCommand,
        ) -> Result<ProcessOutput, unica_process::runner::ProcessStreamFailure> {
            if matches!(self.finish, IncompleteRunnerFinish::SpawnFailure) {
                let stage = self.remember_paths(command);
                if self.create_stage {
                    std::fs::create_dir(&stage).unwrap();
                    std::fs::write(stage.join("foreign.txt"), b"unowned spawn-failure sentinel")
                        .unwrap();
                    self.spawn_output_snapshot
                        .replace(Some(super::capture_tree_target_nofollow(&stage).unwrap()));
                }
                return Err(unica_process::runner::ProcessStreamFailure::Spawn(
                    "the operating system refused to launch the runner".into(),
                ));
            }
            self.run(command)
                .map_err(unica_process::runner::ProcessStreamFailure::Run)
        }
    }

    #[test]
    fn unconfirmed_runner_retains_private_stage_for_late_writer() {
        for finish in [
            IncompleteRunnerFinish::RunError,
            IncompleteRunnerFinish::TimedOut,
            IncompleteRunnerFinish::Cancelled,
            IncompleteRunnerFinish::FailedExit,
        ] {
            for create_stage in [false, true] {
                let (root, context, _) = workspace("unconfirmed-runner-late-writer");
                let target = context.cwd.join("src");
                seed_valid_target(&target);
                let before = super::capture_tree_target_nofollow(&target).unwrap();
                let platform = fixed_platform(&root);
                let runner = IncompleteDumpRunner::new(finish, create_stage);
                let result = VerifiedFullDumpAdapter::with_dependencies(
                    &runner,
                    &platform,
                    PathBuf::from("fake-v8-runner"),
                    crate::infrastructure::runtime_composition::runtime_environment(),
                    crate::infrastructure::runtime_composition::full_dump_semantics(),
                )
                .invoke(
                    "unica.build.dump",
                    FullDumpInvocation::BuildDump,
                    &args(),
                    &context,
                    &CancellationToken::new(),
                )
                .unwrap();
                assert!(!result.ok);
                let (private_root, config_path, stage) =
                    runner.private_paths.borrow().clone().unwrap();
                assert!(
                    private_root.is_dir(),
                    "a live writer lost its attempt directory"
                );
                assert!(
                    std::fs::read(&config_path).unwrap().is_empty(),
                    "held configuration must be empty"
                );
                assert!(result
                    .artifacts
                    .contains(&private_root.display().to_string()));
                runner
                    .pending_writer
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .finish()
                    .unwrap();
                assert_eq!(
                    std::fs::read(stage.join("late-writer.txt")).unwrap(),
                    b"data after runner returned"
                );
                assert_eq!(
                    super::capture_tree_target_nofollow(&target).unwrap(),
                    before
                );
                std::fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn runner_unwind_retains_private_stage_for_late_writer() {
        let (root, context, _) = workspace("runner-unwind-late-writer");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        let platform = fixed_platform(&root);
        let runner = IncompleteDumpRunner::new(IncompleteRunnerFinish::Panic, false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VerifiedFullDumpAdapter::with_dependencies(
                &runner,
                &platform,
                PathBuf::from("fake-v8-runner"),
                crate::infrastructure::runtime_composition::runtime_environment(),
                crate::infrastructure::runtime_composition::full_dump_semantics(),
            )
            .invoke(
                "unica.build.dump",
                FullDumpInvocation::BuildDump,
                &args(),
                &context,
                &CancellationToken::new(),
            )
        }));
        assert!(result.is_err());
        let (private_root, config_path, stage) = runner.private_paths.borrow().clone().unwrap();
        assert!(
            private_root.is_dir(),
            "unwinding deleted a live writer's attempt directory"
        );
        assert!(
            std::fs::read(&config_path).unwrap().is_empty(),
            "held configuration must be empty"
        );
        runner
            .pending_writer
            .borrow_mut()
            .as_mut()
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            std::fs::read(stage.join("late-writer.txt")).unwrap(),
            b"data after runner returned"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn confirmed_spawn_failure_preserves_unexpected_staged_output() {
        let (root, context, _) = workspace("spawn-failure-foreign-output");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        let platform = fixed_platform(&root);
        let runner = IncompleteDumpRunner::new(IncompleteRunnerFinish::SpawnFailure, true);

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &platform,
            PathBuf::from("fake-v8-runner"),
            crate::infrastructure::runtime_composition::runtime_environment(),
            crate::infrastructure::runtime_composition::full_dump_semantics(),
        )
        .invoke(
            "unica.build.dump",
            FullDumpInvocation::BuildDump,
            &args(),
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result.changes.is_empty());
        assert!(result.errors.join("\n").contains("refused to launch"));
        let (private, config, stage) = runner.private_paths.borrow().clone().unwrap();
        assert!(!config.exists());
        assert_eq!(
            super::capture_tree_target_nofollow(&stage).unwrap(),
            *runner.spawn_output_snapshot.borrow().as_ref().unwrap(),
            "a failure to spawn cannot grant ownership of unexpected output"
        );
        assert_eq!(
            std::fs::read(stage.join("foreign.txt")).unwrap(),
            b"unowned spawn-failure sentinel"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        assert_retained_directory_artifact(&result, &private);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_replacement_after_completion_capture_remains_unowned() {
        let (root, context, _) = workspace("staged-replacement-after-completion");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let replacement = root.join("foreign-staged-source");
        std::fs::create_dir(&replacement).unwrap();
        std::fs::write(
            replacement.join("Configuration.xml"),
            valid_configuration_owner(),
        )
        .unwrap();
        std::fs::write(
            replacement.join("Catalogs.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{TARGET_EXPORT_FORMAT}"><Catalog/></MetaDataObject>"#
            ),
        )
        .unwrap();
        let expected_replacement = super::capture_tree_target_nofollow(&replacement).unwrap();
        let displaced = root.join("original-staged-source");
        let retained = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_retained = std::rc::Rc::clone(&retained);
        let hook_workspace = context.cwd.clone();
        let hook_displaced = displaced.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStagedCapture,
            move || {
                let private = std::fs::read_dir(&hook_workspace)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(".unica-dump-guard-")
                    })
                    .unwrap();
                let stage = private.join("execution/staged-source");
                let original = super::capture_tree_target_nofollow(&stage).unwrap();
                std::fs::rename(&stage, &hook_displaced).unwrap();
                std::fs::rename(&replacement, &stage).unwrap();
                *hook_retained.borrow_mut() = Some((private, stage, original));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert!(result.changes.is_empty());
        assert!(
            result
                .errors
                .join("\n")
                .contains("bytes or identity changed"),
            "validation must use the first captured owner identity: {result:?}"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        let retained = retained.borrow();
        let (private, stage, original) = retained.as_ref().expect("capture hook must run");
        assert_eq!(
            super::capture_tree_target_nofollow(stage).unwrap(),
            expected_replacement,
            "cleanup must preserve every identity and byte of the replacement"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&displaced).unwrap(),
            *original,
            "the first observed tree must not be reclassified as the replacement"
        );
        assert_retained_directory_artifact(&result, private);
        std::fs::remove_dir_all(root).unwrap();
    }

    struct EmptySuccessfulDumpRunner;

    impl ProcessRunner for EmptySuccessfulDumpRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            Ok(ProcessOutput {
                status_success: true,
                status: "0".into(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            })
        }
    }

    #[test]
    fn output_appearing_after_absent_completion_capture_remains_unowned() {
        let (root, context, _) = workspace("staged-appearance-after-completion");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        let platform = fixed_platform(&root);
        let retained = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_retained = std::rc::Rc::clone(&retained);
        let hook_workspace = context.cwd.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStagedCapture,
            move || {
                let private = std::fs::read_dir(&hook_workspace)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(".unica-dump-guard-")
                    })
                    .unwrap();
                let stage = private.join("execution/staged-source");
                assert!(!stage.exists());
                std::fs::create_dir(&stage).unwrap();
                std::fs::write(stage.join("Configuration.xml"), valid_configuration_owner())
                    .unwrap();
                std::fs::write(stage.join("foreign.txt"), b"appeared after observation").unwrap();
                let snapshot = super::capture_tree_target_nofollow(&stage).unwrap();
                *hook_retained.borrow_mut() = Some((private, stage, snapshot));
            },
            || {
                VerifiedFullDumpAdapter::with_dependencies(
                    &EmptySuccessfulDumpRunner,
                    &platform,
                    PathBuf::from("fake-v8-runner"),
                    crate::infrastructure::runtime_composition::runtime_environment(),
                    crate::infrastructure::runtime_composition::full_dump_semantics(),
                )
                .invoke(
                    "unica.build.dump",
                    FullDumpInvocation::BuildDump,
                    &args(),
                    &context,
                    &CancellationToken::new(),
                )
                .unwrap()
            },
        );

        assert!(!result.ok, "{result:?}");
        assert!(result.changes.is_empty());
        assert!(result.errors.join("\n").contains("did not create"));
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        let retained = retained.borrow();
        let (private, stage, snapshot) = retained.as_ref().expect("capture hook must run");
        assert_eq!(
            super::capture_tree_target_nofollow(stage).unwrap(),
            *snapshot
        );
        assert_retained_directory_artifact(&result, private);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn confirmed_spawn_failure_cleans_unused_private_stage() {
        let (root, context, _) = workspace("spawn-failure-cleans-stage");
        let platform = fixed_platform(&root);
        let runner = IncompleteDumpRunner::new(IncompleteRunnerFinish::SpawnFailure, false);
        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &platform,
            PathBuf::from("fake-v8-runner"),
            crate::infrastructure::runtime_composition::runtime_environment(),
            crate::infrastructure::runtime_composition::full_dump_semantics(),
        )
        .invoke(
            "unica.build.dump",
            FullDumpInvocation::BuildDump,
            &args(),
            &context,
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(!result.ok);
        let (private_root, config_path, _) = runner.private_paths.borrow().clone().unwrap();
        assert!(!private_root.exists());
        assert!(!config_path.exists());
        assert!(result.artifacts.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_root_creation_is_bound_to_the_captured_target_parent() {
        let (root, context, _) = workspace("private-create-parent-swap");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let visible_workspace = context.cwd.clone();
        let displaced_workspace = root.join("workspace-displaced");
        let hook_visible_workspace = visible_workspace.clone();
        let hook_displaced_workspace = displaced_workspace.clone();

        let result = with_private_create_hook(
            move |_private_root| {
                std::fs::rename(&hook_visible_workspace, &hook_displaced_workspace).unwrap();
                std::fs::create_dir(&hook_visible_workspace).unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        for workspace in [&visible_workspace, &displaced_workspace] {
            assert!(
                std::fs::read_dir(workspace).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".unica-dump-guard-")),
                "private root escaped into a replaced parent: {}",
                workspace.display()
            );
        }
        assert!(
            result
                .errors
                .join("\n")
                .contains("directory anchor identity changed"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unselected_external_source_set_does_not_block_configuration_dump() {
        let (root, context, config) = workspace("configuration-with-external");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        std::fs::create_dir_all(context.cwd.join("external")).unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn applied_full_dump_rejects_ignored_route_arguments_before_runner_execution() {
        let (root, context, _) = workspace("ignored-route-argument");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let mut unsupported = args();
        unsupported.insert(
            "object".to_string(),
            Value::String("Catalog.Items".to_string()),
        );

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &platform,
            PathBuf::from("fake-v8-runner"),
            crate::infrastructure::runtime_composition::runtime_environment(),
            crate::infrastructure::runtime_composition::full_dump_semantics(),
        )
        .invoke(
            "unica.runtime.execute",
            FullDumpInvocation::RuntimeExecute,
            &unsupported,
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result
            .errors
            .join("\n")
            .contains("does not accept `object`"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selected_external_source_set_is_blocked_before_runner_execution() {
        let (root, context, config) = workspace("selected-external");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        std::fs::create_dir_all(context.cwd.join("external")).unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let mut selected = args();
        selected.insert(
            "sourceSet".to_string(),
            Value::String("processors".to_string()),
        );

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &platform,
            PathBuf::from("fake-v8-runner"),
            crate::infrastructure::runtime_composition::runtime_environment(),
            crate::infrastructure::runtime_composition::full_dump_semantics(),
        )
        .invoke(
            "unica.runtime.execute",
            FullDumpInvocation::RuntimeExecute,
            &selected,
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("is external"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn assert_failed_dump_has_no_owned_private_debris(
        context: &WorkspaceContext,
        result: &unica_execution_contract::adapter_outcome::AdapterOutcome,
    ) {
        assert!(
            std::fs::read_dir(&context.cwd).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".unica-dump-guard-")),
            "a finished attempt must discard its own rejected output: {result:?}"
        );
        assert!(
            !result
                .errors
                .iter()
                .any(|error| error.contains("private cleanup")),
            "source rejection must not fabricate an ownership/cleanup error: {result:?}"
        );
        assert!(
            !result
                .warnings
                .iter()
                .any(|warning| warning.contains("debris may remain")),
            "source rejection must not claim retained debris after successful cleanup: {result:?}"
        );
        assert!(
            result.artifacts.is_empty(),
            "a completely discarded private attempt has no retained recovery artifact: {result:?}"
        );
    }

    #[test]
    fn invalid_staged_xml_never_changes_visible_target() {
        for (case, owner_xml, expected) in [
            (
                "older",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#,
                "2.19",
            ),
            (
                "newer",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Configuration/></MetaDataObject>"#,
                "2.21",
            ),
            (
                "entity",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Configuration/></MetaDataObject>"#,
                "2.&#50;0",
            ),
            (
                "malformed",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration>"#,
                "parse",
            ),
        ] {
            let (root, context, _) = workspace(case);
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let platform = fixed_platform(&root);
            let runner = DumpRunner {
                owner_xml: owner_xml.to_string(),
                mutation: RunnerMutation::None,
                calls: AtomicUsize::new(0),
            };

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert!(
                result.errors.join("\n").contains(expected),
                "{case}: {result:?}"
            );
            assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
            assert_eq!(
                std::fs::read_to_string(target.join("Configuration.xml")).unwrap(),
                valid_configuration_owner()
            );
            assert_failed_dump_has_no_owned_private_debris(&context, &result);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn platform_mismatch_blocks_before_runner_execution() {
        let (root, context, _) = workspace("platform-mismatch");
        let runner = DumpRunner::valid();

        let result = invoke(
            &runner,
            &FailingPlatform,
            FullDumpInvocation::BuildDump,
            &context,
        );

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("8.3.26"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(!context.cwd.join("src").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn platform_resolver_does_not_trust_an_explicit_platform_path_or_broad_claim() {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-resolution-{}",
            uuid::Uuid::new_v4()
        ));
        let wrong = root
            .join("8.3.26.9999")
            .join(if cfg!(windows) { "1cv8.exe" } else { "1cv8" });
        make_platform_executable(&wrong);
        let wrong_config: serde_yaml::Value = serde_yaml::from_str(&format!(
            "tools:\n  platform:\n    path: '{}'\n    version: 8.3.27\n",
            wrong.display()
        ))
        .unwrap();

        let error = SystemPlatformResolver
            .resolve(
                &wrong_config,
                &root,
                PlatformUtility::Designer,
                unica_process::runner::system_process_runner(),
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(error.contains("8.3.27"), "{error}");

        let exact = root.join("8.3.27.2074").join("1cv8");
        make_platform_executable(&exact);
        make_platform_executable(&exact.parent().unwrap().join("ibcmd"));
        let exact_config: serde_yaml::Value = serde_yaml::from_str(&format!(
            "tools:\n  platform:\n    path: '{}'\n    version: '8.3'\n",
            exact.display()
        ))
        .unwrap();

        let error = SystemPlatformResolver
            .resolve(
                &exact_config,
                &root,
                PlatformUtility::Designer,
                unica_process::runner::system_process_runner(),
                &CancellationToken::new(),
            )
            .expect_err("a user-owned platform installation must be refused");
        assert!(
            error.contains("immutable")
                || error.contains("owned by root")
                || error.contains("effective root caller"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mutable_platform_swap_and_restore_is_blocked_before_any_child_process() {
        let (root, context, config) = workspace("platform-swap-restore-before-probe");
        let exact = root.join("8.3.27.2074").join("1cv8");
        make_platform_executable(&exact);
        make_platform_executable(&exact.parent().unwrap().join("ibcmd"));
        std::fs::write(
            &config,
            format!(
                "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\ntools:\n  platform:\n    path: '{}'\n    version: '8.3.27'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                exact.display()
            ),
        )
        .unwrap();
        let runner = SwapRestoreProbeRunner {
            install: exact.parent().unwrap().to_path_buf(),
            calls: AtomicUsize::new(0),
        };

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &SystemPlatformResolver,
            PathBuf::from("fake-v8-runner"),
            crate::infrastructure::runtime_composition::runtime_environment(),
            crate::infrastructure::runtime_composition::full_dump_semantics(),
        )
        .invoke(
            "unica.build.dump",
            FullDumpInvocation::BuildDump,
            &args(),
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            runner.calls.load(Ordering::SeqCst),
            0,
            "a mutable installation must be rejected before its probe or v8-runner can swap and restore it"
        );
        assert!(
            result.errors.join("\n").contains("immutable")
                || result.errors.join("\n").contains("owned by root")
                || result.errors.join("\n").contains("effective root caller"),
            "{result:?}"
        );
        assert!(!context.cwd.join("src").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn platform_resolver_rejects_fake_binary_even_under_exact_version_directory_name() {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-fake-resolution-{}",
            uuid::Uuid::new_v4()
        ));
        let fake = root.join("8.3.27.2074/1cv8");
        std::fs::create_dir_all(fake.parent().unwrap()).unwrap();
        std::fs::write(&fake, b"not a platform executable").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let config: serde_yaml::Value = serde_yaml::from_str(&format!(
            "tools:\n  platform:\n    path: '{}'\n    version: '8.3.27'\n",
            fake.display()
        ))
        .unwrap();

        let error = SystemPlatformResolver
            .resolve(
                &config,
                &root,
                PlatformUtility::Designer,
                unica_process::runner::system_process_runner(),
                &CancellationToken::new(),
            )
            .expect_err("a directory name is not platform attestation");

        assert!(
            error.contains("probe") || error.contains("version"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn platform_executable_or_install_swap_after_runner_prevents_publication() {
        for case in ["file", "install"] {
            let (root, context, _) = workspace(&format!("platform-swap-{case}"));
            let target = context.cwd.join("src");
            let platform = fixed_platform(&root);
            let executable = platform.executable.clone();
            let runner = DumpRunner {
                owner_xml: valid_configuration_owner(),
                mutation: match case {
                    "file" => RunnerMutation::PlatformFile(executable),
                    "install" => RunnerMutation::PlatformInstall(
                        platform.executable.parent().unwrap().to_path_buf(),
                    ),
                    _ => unreachable!(),
                },
                calls: AtomicUsize::new(0),
            };

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert!(result.changes.is_empty(), "{case}: {result:?}");
            assert!(!target.join("Configuration.xml").exists(), "{case}");
            assert!(
                result.errors.join("\n").contains("platform")
                    && (result.errors.join("\n").contains("changed")
                        || result.errors.join("\n").contains("attestation")),
                "{case}: {result:?}"
            );
            assert_failed_dump_has_no_owned_private_debris(&context, &result);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn config_and_target_races_block_publication_without_overwriting_concurrent_state() {
        for case in ["config", "target"] {
            let (root, context, config) = workspace(case);
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let platform = fixed_platform(&root);
            let runner = DumpRunner {
                owner_xml: valid_configuration_owner(),
                mutation: match case {
                    "config" => RunnerMutation::Config(config.clone()),
                    "target" => RunnerMutation::Target(target.clone()),
                    _ => unreachable!(),
                },
                calls: AtomicUsize::new(0),
            };

            let result = invoke(
                &runner,
                &platform,
                FullDumpInvocation::RuntimeExecute,
                &context,
            );

            assert!(!result.ok, "{case}: {result:?}");
            assert!(result.changes.is_empty(), "{case}: {result:?}");
            assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
            assert_eq!(
                std::fs::read_to_string(target.join("Configuration.xml")).unwrap(),
                valid_configuration_owner()
            );
            if case == "target" {
                assert_eq!(
                    std::fs::read(target.join("concurrent.txt")).unwrap(),
                    b"concurrent"
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn publication_failure_after_backup_restores_original_tree() {
        let (root, context, _) = workspace("rollback");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = with_publication_failpoint(PublicationCheckpoint::AfterBackup, || {
            invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context)
        });

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("injected"));
        assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
        assert_eq!(
            std::fs::read_to_string(target.join("Configuration.xml")).unwrap(),
            valid_configuration_owner()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_destination_race_after_backup_survives_rollback() {
        let (root, context, _) = workspace("windows-destination-race");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let raced_target = target.clone();
        let hook_workspace = context.cwd.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterBackup,
            move || {
                assert!(
                    !raced_target.exists(),
                    "the original destination must already be moved to backup"
                );
                let private_root = std::fs::read_dir(&hook_workspace)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .expect("private dump root");
                assert!(
                    std::fs::read_dir(private_root.join("recovery"))
                        .unwrap()
                        .flatten()
                        .any(|entry| entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("target-backup-")),
                    "the original destination backup must be retained for rollback"
                );
                std::fs::create_dir_all(&raced_target).unwrap();
                std::fs::write(raced_target.join("racer.txt"), b"racer").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(std::fs::read(target.join("racer.txt")).unwrap(), b"racer");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn backup_name_replacement_is_not_accepted_as_rollback_source() {
        for case in ["snapshot-mismatch", "snapshot-error"] {
            let (root, context, _) = workspace(&format!("rollback-backup-name-replacement-{case}"));
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let displaced_backup = context.cwd.join("validated-backup-displaced");
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_cwd = context.cwd.clone();
            let hook_displaced = displaced_backup.clone();

            let result = with_publication_hook(
                PublicationCheckpoint::BeforeSealedPublishRename,
                move || {
                    let private_root = std::fs::read_dir(&hook_cwd)
                        .unwrap()
                        .flatten()
                        .map(|entry| entry.path())
                        .find(|path| {
                            path.file_name().is_some_and(|name| {
                                name.to_string_lossy().starts_with(".unica-dump-guard-")
                            })
                        })
                        .expect("private dump root");
                    let recovery = private_root.join("recovery");
                    let backup = std::fs::read_dir(&recovery)
                        .unwrap()
                        .flatten()
                        .map(|entry| entry.path())
                        .find(|path| {
                            path.file_name().is_some_and(|name| {
                                name.to_string_lossy().starts_with("target-backup-")
                            })
                        })
                        .expect("validated target backup");
                    std::fs::rename(&backup, &hook_displaced).unwrap();
                    std::fs::create_dir(&backup).unwrap();
                    std::fs::write(
                        backup.join("Configuration.xml"),
                        valid_configuration_owner(),
                    )
                    .unwrap();
                    match case {
                        "snapshot-mismatch" => {
                            std::fs::write(backup.join("unvalidated.txt"), b"unvalidated").unwrap();
                        }
                        "snapshot-error" => {
                            std::os::unix::fs::symlink(
                                backup.join("Configuration.xml"),
                                backup.join("unvalidated-link.xml"),
                            )
                            .unwrap();
                        }
                        _ => unreachable!(),
                    }
                },
                || {
                    with_publication_failpoint(
                        PublicationCheckpoint::BeforeSealedPublishRename,
                        || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
                    )
                },
            );

            assert!(!result.ok, "{case}: {result:?}");
            assert!(
                !target.join("unvalidated.txt").exists()
                    && std::fs::symlink_metadata(target.join("unvalidated-link.xml")).is_err(),
                "{case}: a replacement under the backup name must not remain Git-visible"
            );
            assert!(
                displaced_backup.join("before.txt").is_file(),
                "{case}: the independently displaced original remains untouched"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_target_during_capture_error_quarantine_is_preserved() {
        assert_concurrent_target_survives_quarantine(QuarantineRace::Directory);
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_file_during_capture_error_quarantine_is_preserved() {
        assert_concurrent_target_survives_quarantine(QuarantineRace::File);
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_target_during_capture_error_quarantine_survives_recovery_failure() {
        assert_concurrent_target_survives_quarantine(QuarantineRace::DetachedDirectory);
    }

    #[cfg(unix)]
    enum QuarantineRace {
        Directory,
        File,
        DetachedDirectory,
    }

    #[cfg(unix)]
    fn assert_concurrent_target_survives_quarantine(case: QuarantineRace) {
        let detach_private = matches!(case, QuarantineRace::DetachedDirectory);
        let foreign_file = matches!(case, QuarantineRace::File);
        let (root, context, config) = workspace("quarantine-concurrent-target");
        std::fs::write(&config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=base;Usr=Admin;Pwd=quarantine-secret'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        ).unwrap();
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let displaced_stage = context.cwd.join("externally-displaced-stage");
        let detached_private = context.cwd.join("externally-displaced-private");
        let hook_target = std::fs::canonicalize(&target).unwrap();
        let probe_target = target.clone();
        let hook_displaced = displaced_stage.clone();
        let hook_private = detached_private.clone();
        let hook_cwd = context.cwd.clone();
        let armed = std::rc::Rc::new(std::cell::Cell::new(false));
        let probe_armed = armed.clone();
        let identity = std::rc::Rc::new(std::cell::Cell::new(None));
        let hook_identity = identity.clone();
        let result = super::with_directory_rename_hook(
            move |source| {
                if source != hook_target || !armed.replace(false) {
                    return;
                }
                std::fs::rename(&hook_target, &hook_displaced).unwrap();
                let foreign = if foreign_file {
                    std::fs::write(&hook_target, b"concurrent").unwrap();
                    unica_workspace::test_support::read_file_from_fixture_file(
                        std::fs::File::open(&hook_target).unwrap(),
                    )
                } else {
                    std::fs::create_dir(&hook_target).unwrap();
                    std::fs::write(hook_target.join("concurrent.txt"), b"concurrent").unwrap();
                    super::open_directory_nofollow(&hook_target).unwrap()
                };
                hook_identity.set(Some(super::file_identity(&foreign).unwrap()));
                if detach_private {
                    let private = std::fs::read_dir(&hook_cwd)
                        .unwrap()
                        .filter_map(Result::ok)
                        .map(|entry| entry.path())
                        .find(|path| {
                            path.file_name().is_some_and(|name| {
                                name.to_string_lossy().starts_with(".unica-dump-guard-")
                            })
                        })
                        .unwrap();
                    std::fs::rename(private, &hook_private).unwrap();
                }
            },
            || {
                with_publication_hook(
                    PublicationCheckpoint::BeforeInstalledSnapshot,
                    move || {
                        std::os::unix::fs::symlink(
                            probe_target.join("Configuration.xml"),
                            probe_target.join("unvalidated-link.xml"),
                        )
                        .unwrap();
                        probe_armed.set(true);
                    },
                    || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
                )
            },
        );
        assert!(!result.ok, "{detach_private}: {result:?}");
        let contents = |path: &Path| {
            std::fs::read(if foreign_file {
                path.to_path_buf()
            } else {
                path.join("concurrent.txt")
            })
        };
        let foreign = if contents(&target).is_ok_and(|bytes| bytes == b"concurrent") {
            target.clone()
        } else {
            let private = if detach_private {
                detached_private.clone()
            } else {
                std::fs::read_dir(&context.cwd)
                    .unwrap()
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .expect("foreign target must survive in retained recovery")
            };
            std::fs::read_dir(private.join("recovery"))
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| contents(path).is_ok_and(|bytes| bytes == b"concurrent"))
                .expect("foreign target must not be deleted during cleanup")
        };
        assert_eq!(contents(&foreign).unwrap(), b"concurrent");
        let foreign_handle = if foreign_file {
            unica_workspace::test_support::read_file_from_fixture_file(
                std::fs::File::open(&foreign).unwrap(),
            )
        } else {
            super::open_directory_nofollow(&std::fs::canonicalize(&foreign).unwrap()).unwrap()
        };
        assert_eq!(
            Some(super::file_identity(&foreign_handle).unwrap()),
            identity.get()
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        let private = if detach_private {
            detached_private
        } else {
            std::fs::read_dir(&context.cwd)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name().is_some_and(|name| {
                        name.to_string_lossy().starts_with(".unica-dump-guard-")
                    })
                })
                .expect("owned original backup remains recoverable")
        };
        if detach_private {
            let config = private.join("execution").join(super::EFFECTIVE_CONFIG_NAME);
            assert!(
                !config.exists() || std::fs::read(config).unwrap().is_empty(),
                "{result:?}"
            );
        } else {
            assert!(!private.join("execution").exists(), "{result:?}");
        }
        assert!(std::fs::read_dir(private.join("recovery"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.path().join("before.txt").is_file()));
        let retained_root_identity = super::file_identity(
            &super::open_directory_nofollow(&std::fs::canonicalize(&private).unwrap()).unwrap(),
        )
        .unwrap();
        let retained_recovery_identity = super::file_identity(
            &super::open_directory_nofollow(
                &std::fs::canonicalize(private.join("recovery")).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let owned_artifacts = result.artifacts.iter().all(|artifact| {
            std::fs::canonicalize(artifact)
                .ok()
                .and_then(|path| super::open_directory_nofollow(&path).ok())
                .and_then(|directory| super::file_identity(&directory).ok())
                .is_some_and(|identity| {
                    identity == retained_root_identity || identity == retained_recovery_identity
                })
        });
        let location_unconfirmed = result
            .warnings
            .iter()
            .any(|warning| warning.contains("current location is unconfirmed"));
        assert!(
            owned_artifacts && (!result.artifacts.is_empty() || location_unconfirmed),
            "quarantine recovery must report an owned identity or explicit uncertainty"
        );
        if detach_private {
            assert!(
                result.artifacts.is_empty() && location_unconfirmed,
                "a displaced private root must not advertise its missing original name"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_target_directory_before_first_installed_probe_survives_rollback() {
        for had_target in [true, false] {
            assert_foreign_target_survives_first_installed_probe(had_target, "directory");
        }
    }

    #[test]
    fn concurrent_target_file_before_first_installed_probe_survives_rollback() {
        for had_target in [true, false] {
            assert_foreign_target_survives_first_installed_probe(had_target, "file");
        }
    }

    #[test]
    fn absent_target_before_first_installed_probe_restores_the_owned_backup() {
        let (root, context, _) = workspace("first-probe-absent-target");
        let target = context.cwd.join("src");
        let displaced_stage = context.cwd.join("externally-displaced-stage");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();
        let hook_displaced_stage = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::BeforeInstalledSnapshot,
            move || std::fs::rename(&hook_target, &hook_displaced_stage).unwrap(),
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
        assert!(displaced_stage.join("Configuration.xml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn assert_foreign_target_survives_first_installed_probe(had_target: bool, kind: &str) {
        let (root, context, _) = workspace(&format!("first-probe-{kind}-{had_target}"));
        let target = context.cwd.join("src");
        let displaced_stage = context.cwd.join("externally-displaced-stage");
        if had_target {
            seed_valid_target(&target);
        }
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();
        let hook_displaced_stage = displaced_stage.clone();
        let foreign_identity = std::rc::Rc::new(std::cell::Cell::new(None));
        let hook_identity = foreign_identity.clone();
        let is_directory = kind == "directory";

        let result = with_publication_hook(
            PublicationCheckpoint::BeforeInstalledSnapshot,
            move || {
                std::fs::rename(&hook_target, &hook_displaced_stage).unwrap();
                let foreign = if is_directory {
                    std::fs::create_dir(&hook_target).unwrap();
                    std::fs::write(hook_target.join("concurrent.txt"), b"concurrent").unwrap();
                    super::open_directory_nofollow(&std::fs::canonicalize(&hook_target).unwrap())
                        .unwrap()
                } else {
                    std::fs::write(&hook_target, b"concurrent").unwrap();
                    unica_workspace::test_support::read_file_from_fixture_file(
                        std::fs::File::open(&hook_target).unwrap(),
                    )
                };
                hook_identity.set(Some(super::file_identity(&foreign).unwrap()));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{kind}/{had_target}: {result:?}");
        let contents = if is_directory {
            std::fs::read(target.join("concurrent.txt"))
        } else {
            std::fs::read(&target)
        };
        assert_eq!(
            contents.unwrap_or_else(|error| panic!(
                "{kind}/{had_target}: publication removed the concurrent target: {error}; {result:?}"
            )),
            b"concurrent"
        );
        let foreign = if is_directory {
            super::open_directory_nofollow(&std::fs::canonicalize(&target).unwrap()).unwrap()
        } else {
            unica_workspace::test_support::read_file_from_fixture_file(
                std::fs::File::open(&target).unwrap(),
            )
        };
        assert_eq!(
            Some(super::file_identity(&foreign).unwrap()),
            foreign_identity.get(),
            "{kind}/{had_target}: publication moved or replaced the concurrent target"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        let recoveries = std::fs::read_dir(&context.cwd)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .collect::<Vec<_>>();
        assert_eq!(recoveries.len(), 1, "{result:?}");
        assert!(!recoveries[0].join("execution").exists());
        let recovery = std::fs::canonicalize(recoveries[0].join("recovery")).unwrap();
        assert_retained_directory_artifact(&result, &recovery);
        assert!(
            result.summary.contains("publication state is unknown"),
            "{result:?}"
        );
        if had_target {
            let backup = std::fs::read_dir(&recovery)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("target-backup-"))
                })
                .expect("the owned original remains available for recovery");
            assert_eq!(std::fs::read(backup.join("before.txt")).unwrap(), b"before");
        }
        drop(foreign);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_target_replacement_after_stage_install_survives_rollback() {
        let (root, context, _) = workspace("rollback-concurrent-after-install");
        let target = context.cwd.join("src");
        let displaced_stage = context.cwd.join("externally-displaced-stage");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();
        let hook_displaced_stage = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::rename(&hook_target, &hook_displaced_stage).unwrap();
                std::fs::create_dir_all(&hook_target).unwrap();
                std::fs::write(hook_target.join("concurrent.txt"), b"concurrent").unwrap();
            },
            || {
                with_publication_failpoint(PublicationCheckpoint::AfterStageInstall, || {
                    invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context)
                })
            },
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("concurrent.txt")).unwrap(),
            b"concurrent",
            "rollback must never move or overwrite a target it no longer owns"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_tree_interior_mutation_is_quarantined_and_rolled_back() {
        for checkpoint in [
            PublicationCheckpoint::BeforeInstalledSnapshot,
            PublicationCheckpoint::AfterStageInstall,
        ] {
            let (root, context, _) = workspace("installed-tree-interior-mutation");
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_target = target.clone();

            let result = with_publication_hook(
                checkpoint,
                move || {
                    std::fs::write(hook_target.join("unvalidated.txt"), b"unvalidated").unwrap();
                },
                || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
            );

            assert!(!result.ok, "{result:?}");
            assert_eq!(
                std::fs::read(target.join("before.txt")).unwrap(),
                b"before",
                "the original target must be restored after mutation of our installed directory"
            );
            assert!(
                !target.join("unvalidated.txt").exists(),
                "our installed but mutated directory must be quarantined before lock release"
            );
            assert!(
                result.errors.join("\n").contains("published dump differs")
                    || result.errors.join("\n").contains("quarantine"),
                "{result:?}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn installed_tree_capture_error_with_same_root_identity_is_quarantined_and_rolled_back() {
        for checkpoint in [
            PublicationCheckpoint::BeforeInstalledSnapshot,
            PublicationCheckpoint::AfterStageInstall,
        ] {
            let (root, context, _) = workspace("installed-tree-interior-symlink");
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_target = target.clone();

            let result = with_publication_hook(
                checkpoint,
                move || {
                    std::os::unix::fs::symlink(
                        hook_target.join("Configuration.xml"),
                        hook_target.join("unvalidated-link.xml"),
                    )
                    .unwrap();
                },
                || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
            );

            assert!(!result.ok, "{result:?}");
            assert_eq!(
                std::fs::read(target.join("before.txt")).unwrap(),
                b"before",
                "the original target must be restored after capture failure in our installed directory"
            );
            assert!(
                std::fs::symlink_metadata(target.join("unvalidated-link.xml")).is_err(),
                "our installed but invalid directory must be quarantined before lock release"
            );
            assert!(
                result
                    .errors
                    .join("\n")
                    .contains("could not verify the published target")
                    || result.errors.join("\n").contains("quarantine"),
                "{result:?}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn stage_name_replacement_before_publish_never_becomes_visible() {
        let (root, context, _) = workspace("stage-name-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let displaced_stage = context.cwd.join("validated-stage-displaced");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_cwd = context.cwd.clone();
        let hook_displaced = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::BeforeStageInstall,
            move || {
                let private_root = std::fs::read_dir(&hook_cwd)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .expect("private dump root");
                let stage = private_root.join("execution/staged-source");
                std::fs::rename(&stage, &hook_displaced).unwrap();
                std::fs::create_dir_all(&stage).unwrap();
                std::fs::write(stage.join("Configuration.xml"), valid_configuration_owner())
                    .unwrap();
                std::os::unix::fs::symlink(
                    stage.join("Configuration.xml"),
                    stage.join("unvalidated-link.xml"),
                )
                .unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("before.txt")).unwrap(),
            b"before",
            "the original target must be restored without exposing the replacement stage"
        );
        assert!(
            !target.join("unvalidated-link.xml").exists(),
            "the replacement stage must never remain Git-visible"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reappeared_execution_stage_survives_success_and_rollback() {
        assert_reappeared_private_stage_is_preserved(true);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_open_replacement_preserves_owned_and_foreign_trees() {
        assert_cleanup_open_preserves_foreign_data(false);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_open_preserves_unknown_descendants_of_owned_tree() {
        assert_cleanup_open_preserves_foreign_data(true);
    }

    #[cfg(unix)]
    fn assert_cleanup_open_preserves_foreign_data(nested: bool) {
        let (root, context, _) = workspace("cleanup-open-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let original = super::capture_tree_target_nofollow(&target).unwrap();
        let displaced = context.cwd.join("displaced-owned-backup");
        let hook_displaced = displaced.clone();
        let foreign = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_foreign = foreign.clone();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let result = super::with_directory_cleanup_open_hook(
            move |path| {
                if hook_foreign.borrow().is_some()
                    || !path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("target-backup-"))
                {
                    return;
                }
                let foreign_path = if nested {
                    let nested_path = path.join("unknown-child");
                    std::fs::create_dir(&nested_path).unwrap();
                    nested_path
                } else {
                    std::fs::rename(path, &hook_displaced).unwrap();
                    std::fs::create_dir(path).unwrap();
                    path.to_path_buf()
                };
                std::fs::write(foreign_path.join("sentinel.txt"), b"foreign cleanup data").unwrap();
                let snapshot = super::capture_tree_target_nofollow(&foreign_path).unwrap();
                hook_foreign.replace(Some((foreign_path, snapshot)));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );
        let (foreign_path, foreign_snapshot) = foreign.borrow().clone().expect("cleanup hook ran");
        assert_eq!(
            super::capture_tree_target_nofollow(&foreign_path).unwrap(),
            foreign_snapshot,
            "cleanup must preserve a foreign root and all its data"
        );
        if !nested {
            assert_eq!(
                super::capture_tree_target_nofollow(&displaced).unwrap(),
                original
            );
        }
        assert!(std::fs::read_to_string(target.join("Configuration.xml"))
            .unwrap()
            .contains(TARGET_EXPORT_FORMAT));
        assert!(!result.ok, "{result:?}");
        assert!(
            result.summary.contains("private cleanup failed"),
            "{result:?}"
        );
        assert!(
            !result.summary.contains("publication state is unknown"),
            "{result:?}"
        );
        assert!(!result.artifacts.is_empty(), "{result:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reappeared_sealed_stage_survives_success_and_rollback() {
        assert_reappeared_private_stage_is_preserved(false);
    }

    #[cfg(unix)]
    fn assert_reappeared_private_stage_is_preserved(execution_slot: bool) {
        for rollback in [false, true] {
            let (root, context, _) = workspace("reappeared-private-stage");
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let original = super::capture_tree_target_nofollow(&target).unwrap();
            let staged = std::rc::Rc::new(std::cell::RefCell::new(None));
            let hook_staged = staged.clone();
            let checkpoint_staged = staged.clone();
            let sealed_path = std::rc::Rc::new(std::cell::RefCell::new(None));
            let rename_sealed_path = sealed_path.clone();
            let hook_sealed_path = sealed_path.clone();
            let foreign = std::rc::Rc::new(std::cell::RefCell::new(None));
            let hook_foreign = foreign.clone();
            let checkpoint = if execution_slot {
                PublicationCheckpoint::AfterBackup
            } else {
                PublicationCheckpoint::AfterStageInstall
            };
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_cwd = context.cwd.clone();
            let result = super::with_directory_rename_hook(
                move |source| {
                    if source
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("sealed-stage-"))
                        && hook_staged.borrow().is_none()
                    {
                        hook_staged
                            .replace(Some(super::capture_tree_target_nofollow(source).unwrap()));
                        rename_sealed_path.replace(Some(source.to_path_buf()));
                    }
                },
                || {
                    with_publication_hook(
                        checkpoint,
                        move || {
                            let sealed: PathBuf =
                                hook_sealed_path.borrow().clone().unwrap_or_else(|| {
                                    let private = std::fs::read_dir(&hook_cwd)
                                        .unwrap()
                                        .filter_map(Result::ok)
                                        .map(|entry| entry.path())
                                        .find(|path| {
                                            path.file_name().is_some_and(|name| {
                                                name.to_string_lossy()
                                                    .starts_with(".unica-dump-guard-")
                                            })
                                        })
                                        .unwrap();
                                    std::fs::read_dir(private.join("recovery"))
                                        .unwrap()
                                        .filter_map(Result::ok)
                                        .map(|entry| entry.path())
                                        .find(|path| {
                                            path.file_name().is_some_and(|name| {
                                                name.to_string_lossy().starts_with("sealed-stage-")
                                            })
                                        })
                                        .unwrap()
                                });
                            if checkpoint_staged.borrow().is_none() {
                                checkpoint_staged.replace(Some(
                                    super::capture_tree_target_nofollow(&sealed).unwrap(),
                                ));
                            }
                            hook_sealed_path.replace(Some(sealed.clone()));
                            let path = if execution_slot {
                                sealed
                                    .parent()
                                    .unwrap()
                                    .parent()
                                    .unwrap()
                                    .join("execution/staged-source")
                            } else {
                                sealed
                            };
                            std::fs::create_dir(&path).unwrap();
                            std::fs::write(path.join("foreign.txt"), b"reappeared private stage")
                                .unwrap();
                            hook_foreign.replace(Some((
                                path.clone(),
                                super::capture_tree_target_nofollow(&path).unwrap(),
                            )));
                        },
                        || {
                            if rollback {
                                with_publication_failpoint(checkpoint, || {
                                    invoke(
                                        &runner,
                                        &platform,
                                        FullDumpInvocation::BuildDump,
                                        &context,
                                    )
                                })
                            } else {
                                invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context)
                            }
                        },
                    )
                },
            );
            let (foreign_path, foreign_snapshot) = foreign.borrow().as_ref().unwrap().clone();
            assert_eq!(
                super::capture_tree_target_nofollow(&foreign_path).unwrap(),
                foreign_snapshot,
                "a reused private slot must retain the foreign root identity and bytes"
            );
            let private = sealed_path
                .borrow()
                .as_ref()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf();
            let mut snapshots = vec![super::capture_tree_target_nofollow(&target).unwrap()];
            for directory in [private.join("execution"), private.join("recovery")] {
                if directory.is_dir() {
                    snapshots.extend(
                        std::fs::read_dir(directory)
                            .unwrap()
                            .filter_map(Result::ok)
                            .filter(|entry| entry.file_type().unwrap().is_dir())
                            .map(|entry| {
                                super::capture_tree_target_nofollow(&entry.path()).unwrap()
                            }),
                    );
                }
            }
            assert!(
                snapshots.contains(&original),
                "original source must remain recoverable"
            );
            assert!(
                snapshots.contains(staged.borrow().as_ref().unwrap()),
                "validated stage must remain recoverable"
            );
            assert!(!result.ok, "{execution_slot}/{rollback}: {result:?}");
            if execution_slot {
                let config = private.join("execution").join(super::EFFECTIVE_CONFIG_NAME);
                assert!(!config.exists() || std::fs::read(config).unwrap().is_empty());
            } else {
                assert!(!private.join("execution").exists());
            }
            if rollback {
                assert_eq!(
                    super::capture_tree_target_nofollow(&target).unwrap(),
                    original
                );
                assert!(
                    !result.summary.contains("publication state is unknown"),
                    "{result:?}"
                );
            } else {
                assert!(
                    result
                        .summary
                        .contains("validated publication completed but private cleanup failed"),
                    "{result:?}"
                );
                assert!(
                    !result.summary.contains("publication state is unknown"),
                    "{result:?}"
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_stage_swap_before_seal_precheck_preserves_data_and_scrubs_credentials() {
        assert_private_stage_swap_during_seal_is_preserved(true);
    }

    #[cfg(unix)]
    #[test]
    fn private_stage_swap_after_seal_precheck_preserves_data_and_scrubs_credentials() {
        assert_private_stage_swap_during_seal_is_preserved(false);
    }

    #[cfg(unix)]
    fn assert_private_stage_swap_during_seal_is_preserved(before_precheck: bool) {
        for had_target in [true, false] {
            let (root, context, config) = workspace("private-stage-seal-swap");
            std::fs::write(&config,
                "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=base;Usr=Admin;Pwd=seal-secret'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            ).unwrap();
            let target = std::fs::canonicalize(&context.cwd).unwrap().join("src");
            if had_target {
                seed_valid_target(&target);
            }
            let original = super::capture_tree_target_nofollow(&target).unwrap();
            let displaced_stage = context.cwd.join("externally-displaced-private-stage");
            let hook_displaced = displaced_stage.clone();
            let stage_before = std::rc::Rc::new(std::cell::RefCell::new(None));
            let hook_stage = stage_before.clone();
            let foreign_before = std::rc::Rc::new(std::cell::RefCell::new(None));
            let hook_foreign = foreign_before.clone();
            let swap = move |source: &Path| {
                if hook_stage.borrow().is_some() {
                    return;
                }
                hook_stage.replace(Some(super::capture_tree_target_nofollow(source).unwrap()));
                std::fs::rename(source, &hook_displaced).unwrap();
                std::fs::create_dir(source).unwrap();
                std::fs::write(
                    source.join("Configuration.xml"),
                    b"foreign private-stage XML",
                )
                .unwrap();
                std::fs::write(source.join("foreign.txt"), b"foreign private-stage data").unwrap();
                hook_foreign.replace(Some(super::capture_tree_target_nofollow(source).unwrap()));
            };
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_cwd = context.cwd.clone();
            let result = if before_precheck {
                with_publication_hook(
                    PublicationCheckpoint::BeforeStageInstall,
                    move || {
                        let private = std::fs::read_dir(&hook_cwd)
                            .unwrap()
                            .filter_map(Result::ok)
                            .map(|entry| entry.path())
                            .find(|path| {
                                path.file_name().is_some_and(|name| {
                                    name.to_string_lossy().starts_with(".unica-dump-guard-")
                                })
                            })
                            .unwrap();
                        swap(&private.join("execution/staged-source"));
                    },
                    || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
                )
            } else {
                super::with_directory_rename_hook(
                    move |source| {
                        if source
                            .file_name()
                            .is_some_and(|name| name == "staged-source")
                        {
                            swap(source);
                        }
                    },
                    || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
                )
            };
            assert!(!result.ok, "{before_precheck}/{had_target}: {result:?}");
            assert_eq!(
                super::capture_tree_target_nofollow(&target).unwrap(),
                original
            );
            assert_eq!(
                Some(super::capture_tree_target_nofollow(&displaced_stage).unwrap()),
                *stage_before.borrow()
            );
            let private = std::fs::read_dir(&context.cwd)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name().is_some_and(|name| {
                        name.to_string_lossy().starts_with(".unica-dump-guard-")
                    })
                })
                .expect("uncertain stage must survive private cleanup");
            let mut snapshots = Vec::new();
            for directory in [private.join("execution"), private.join("recovery")] {
                for entry in std::fs::read_dir(directory).unwrap().filter_map(Result::ok) {
                    if entry.file_type().unwrap().is_dir() {
                        snapshots.push(super::capture_tree_target_nofollow(&entry.path()).unwrap());
                    }
                }
            }
            assert!(
                snapshots.contains(
                    foreign_before
                        .borrow()
                        .as_ref()
                        .expect("source swap occurred")
                ),
                "foreign private stage identity and bytes must be retained"
            );
            let config = private.join("execution").join(super::EFFECTIVE_CONFIG_NAME);
            assert!(!config.exists() || std::fs::read(config).unwrap().is_empty(),
                "the held effective config must be scrubbed without deleting unknown stage contents");
            assert!(
                result
                    .summary
                    .contains("failed before verified dump publication"),
                "{result:?}"
            );
            assert!(
                result.artifacts.contains(
                    &std::fs::canonicalize(&private)
                        .unwrap()
                        .display()
                        .to_string()
                ),
                "{result:?}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn sealed_source_swap_after_identity_precheck_preserves_data_and_reports_unknown() {
        assert_sealed_source_swap_after_precheck_reports_unknown(true);
    }

    #[cfg(unix)]
    #[test]
    fn sealed_source_swap_after_identity_precheck_without_original_reports_unknown() {
        assert_sealed_source_swap_after_precheck_reports_unknown(false);
    }

    #[cfg(unix)]
    fn assert_sealed_source_swap_after_precheck_reports_unknown(had_target: bool) {
        let (root, context, _) = workspace("sealed-source-after-precheck");
        let target = std::fs::canonicalize(&context.cwd).unwrap().join("src");
        if had_target {
            seed_valid_target(&target);
        }
        let original = had_target.then(|| super::capture_tree_target_nofollow(&target).unwrap());
        let displaced_stage = context.cwd.join("externally-displaced-validated-stage");
        let hook_displaced = displaced_stage.clone();
        let stage_before = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_stage_before = stage_before.clone();
        let foreign_before = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_foreign_before = foreign_before.clone();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = super::with_directory_rename_hook(
            move |source| {
                if !source
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("sealed-stage-"))
                    || hook_stage_before.borrow().is_some()
                {
                    return;
                }
                hook_stage_before
                    .replace(Some(super::capture_tree_target_nofollow(source).unwrap()));
                std::fs::rename(source, &hook_displaced).unwrap();
                std::fs::create_dir(source).unwrap();
                std::fs::write(source.join("Configuration.xml"), b"unvalidated XML owner").unwrap();
                std::fs::write(source.join("unvalidated.txt"), b"foreign sealed source").unwrap();
                hook_foreign_before
                    .replace(Some(super::capture_tree_target_nofollow(source).unwrap()));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            Some(
                super::capture_tree_target_nofollow(
                    &std::fs::canonicalize(&displaced_stage).unwrap()
                )
                .unwrap()
            ),
            *stage_before.borrow(),
            "the held validated stage must retain its identity and bytes"
        );
        let private = std::fs::read_dir(&context.cwd)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .expect("owned original and unverified source must remain recoverable");
        let mut snapshots = vec![super::capture_tree_target_nofollow(&target).unwrap()];
        snapshots.extend(
            std::fs::read_dir(private.join("recovery"))
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| super::capture_tree_target_nofollow(&entry.path()).unwrap()),
        );
        if let Some(original) = original {
            assert!(
                snapshots.contains(&original),
                "original target identity and bytes must be retained"
            );
        }
        assert!(
            snapshots.contains(
                foreign_before
                    .borrow()
                    .as_ref()
                    .expect("source swap occurred")
            ),
            "foreign sealed source identity and bytes must be retained"
        );
        assert!(!private.join("execution").exists());
        let recovery = std::fs::canonicalize(private.join("recovery")).unwrap();
        assert!(
            result.artifacts.contains(&recovery.display().to_string()),
            "{result:?}"
        );
        assert!(
            result.summary.contains("publication state is unknown"),
            "{result:?}"
        );
        assert!(
            !result
                .summary
                .contains("failed before verified dump publication"),
            "{result:?}"
        );
        assert!(
            result
                .warnings
                .iter()
                .all(|warning| !warning.contains("discarded")
                    && !warning.contains("sources were not published")),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn sealed_stage_replacement_at_publish_boundary_is_quarantined_before_lock_release() {
        let (root, context, _) = workspace("sealed-stage-publish-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let displaced_stage = context.cwd.join("sealed-validated-stage-displaced");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_cwd = context.cwd.clone();
        let hook_displaced = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::BeforeSealedPublishRename,
            move || {
                let private_root = std::fs::read_dir(&hook_cwd)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .expect("private dump root");
                let recovery = private_root.join("recovery");
                let sealed = std::fs::read_dir(&recovery)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name()
                            .is_some_and(|name| name.to_string_lossy().starts_with("sealed-stage-"))
                    })
                    .expect("sealed staged dump");
                std::fs::rename(&sealed, &hook_displaced).unwrap();
                std::fs::create_dir(&sealed).unwrap();
                std::fs::write(
                    sealed.join("Configuration.xml"),
                    valid_configuration_owner(),
                )
                .unwrap();
                std::fs::write(sealed.join("unvalidated.txt"), b"unvalidated").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("before.txt")).unwrap(),
            b"before",
            "the original target must be restored before the publication lock is released"
        );
        assert!(
            !target.join("unvalidated.txt").exists(),
            "the source-name replacement must be removed from the Git-visible target"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        assert!(
            result.errors.join("\n").contains("just-installed")
                || result.errors.join("\n").contains("unverified")
                || result.errors.join("\n").contains("quarantine")
                || result
                    .errors
                    .join("\n")
                    .contains("identity changed before atomic rename"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preserved_recovery_never_contains_effective_config_or_credentials() {
        let (root, context, config) = workspace("recovery-secrets");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=base;Usr=Admin;Pwd=top-secret'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let displaced_stage = context.cwd.join("secret-test-displaced-stage");
        let platform = fixed_platform(&root);
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::None,
            calls: AtomicUsize::new(0),
        };
        let hook_target = target.clone();
        let hook_displaced_stage = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::rename(&hook_target, &hook_displaced_stage).unwrap();
                std::fs::create_dir_all(&hook_target).unwrap();
                std::fs::write(hook_target.join("concurrent.txt"), b"concurrent").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        let recovery_roots = std::fs::read_dir(&context.cwd)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .collect::<Vec<_>>();
        assert!(!recovery_roots.is_empty(), "{result:?}");
        for recovery in recovery_roots {
            let snapshot = super::capture_directory_snapshot(&recovery).unwrap();
            assert!(
                snapshot.iter().all(|entry| {
                    entry
                        .relative_path
                        .file_name()
                        .is_none_or(|name| name != super::EFFECTIVE_CONFIG_NAME)
                }),
                "effective config leaked into recovery: {}",
                recovery.display()
            );
            for entry in snapshot {
                if let super::TreeEntryKind::File { .. } = entry.kind {
                    let raw = std::fs::read(recovery.join(entry.relative_path)).unwrap();
                    assert!(
                        !String::from_utf8_lossy(&raw).contains("top-secret"),
                        "credentials leaked into recovery: {}",
                        recovery.display()
                    );
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_root_swap_cannot_escape_secret_scrubbing_or_anchor_cleanup() {
        let (root, context, config) = workspace("private-root-swap-cleanup");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=base;Usr=Admin;Pwd=top-secret'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let platform = fixed_platform(&root);
        let original_identity = std::rc::Rc::new(std::cell::Cell::new(None));
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::SwapPrivateRoot(std::rc::Rc::clone(&original_identity)),
            calls: AtomicUsize::new(0),
        };

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
        assert!(!context.cwd.join("src").exists(), "{result:?}");
        let private_roots = std::fs::read_dir(&context.cwd)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .collect::<Vec<_>>();
        assert!(
            private_roots.iter().any(|path| path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("-displaced")),
            "{private_roots:?}; {result:?}"
        );
        for private_root in &private_roots {
            let snapshot = super::capture_directory_snapshot(private_root).unwrap();
            assert!(
                snapshot.iter().all(|entry| {
                    entry
                        .relative_path
                        .file_name()
                        .is_none_or(|name| name != super::EFFECTIVE_CONFIG_NAME)
                }),
                "effective config escaped cleanup: {}",
                private_root.display()
            );
            for entry in snapshot {
                if let super::TreeEntryKind::File { .. } = entry.kind {
                    let raw = std::fs::read(private_root.join(entry.relative_path)).unwrap();
                    assert!(
                        !String::from_utf8_lossy(&raw).contains("top-secret"),
                        "credentials escaped cleanup: {}",
                        private_root.display()
                    );
                }
            }
        }
        assert!(
            result.errors.join("\n").contains("identity changed")
                || result.warnings.join("\n").contains("debris"),
            "{result:?}"
        );
        let original_identity = original_identity.get().expect("root swap must run");
        let displaced = private_roots
            .iter()
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with("-displaced")
            })
            .unwrap();
        assert_eq!(
            super::file_identity(
                &super::open_directory_nofollow(&std::fs::canonicalize(displaced).unwrap())
                    .unwrap()
            )
            .unwrap(),
            original_identity
        );
        let replacement = private_roots
            .iter()
            .find(|path| {
                !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with("-displaced")
            })
            .unwrap();
        assert_ne!(
            super::file_identity(
                &super::open_directory_nofollow(&std::fs::canonicalize(replacement).unwrap())
                    .unwrap()
            )
            .unwrap(),
            original_identity
        );
        assert_eq!(
            std::fs::read(replacement.join("execution/foreign.txt")).unwrap(),
            b"foreign root sentinel"
        );
        let owned_artifacts = result.artifacts.iter().all(|artifact| {
            std::fs::canonicalize(artifact)
                .ok()
                .and_then(|path| super::open_directory_nofollow(&path).ok())
                .and_then(|directory| super::file_identity(&directory).ok())
                == Some(original_identity)
        });
        let location_unconfirmed = result
            .warnings
            .iter()
            .any(|warning| warning.contains("current location is unconfirmed"));
        assert!(owned_artifacts && (!result.artifacts.is_empty() || location_unconfirmed),
            "recovery must report an owned identity or explicit uncertainty: owned={owned_artifacts}, unconfirmed={location_unconfirmed}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_recovery_artifacts_require_the_retained_directory_identity() {
        unica_workspace::full_dump::test_support::retained_recovery_identity_contract(
            workspace,
            || {
                super::dump_failure(
                    "unica.build.dump",
                    "fixture caller refused".into(),
                    None,
                    None,
                    None,
                )
            },
            super::finalize_private_outcome,
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_stage_cleanup_failures_are_reported() {
        use std::os::unix::fs::PermissionsExt;

        let (root, context, _) = workspace("stage-remove-error");
        let platform = fixed_platform(&root);
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::RestrictPrivateRoot,
            calls: AtomicUsize::new(0),
        };

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(!result.ok, "{result:?}");
        assert!(
            result.errors.join("\n").contains("cleanup failed")
                || result.warnings.join("\n").contains("cleanup failed"),
            "{result:?}"
        );
        for entry in std::fs::read_dir(&context.cwd).unwrap().flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".unica-dump-guard-")
            {
                std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                let execution = entry.path().join("execution");
                if execution.exists() {
                    std::fs::set_permissions(execution, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_in_private_stage_is_rejected_before_publication() {
        let (root, context, _) = workspace("stage-symlink");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::Symlink,
            calls: AtomicUsize::new(0),
        };

        let result = invoke(
            &runner,
            &platform,
            FullDumpInvocation::RuntimeExecute,
            &context,
        );

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("symbolic link"));
        assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owner_kind_is_bound_to_selected_source_set() {
        let root =
            std::env::temp_dir().join(format!("unica-dump-owner-kind-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();

        let error = validate_staged_dump(&root, SourceSetKind::Extension).unwrap_err();

        assert!(error.contains("without ConfigurationExtensionPurpose"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_tree_uses_closed_root_registry_and_exact_raw_versions() {
        let cases = [
            (
                "known-missing",
                "Forms/Main/Ext/Form.xml",
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"/>"#,
                "missing",
            ),
            (
                "known-numeric-equivalent",
                "Forms/Main/Ext/Form.xml",
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.200"/>"#,
                "2.200",
            ),
            (
                "unknown-version-bearing",
                "Unknown.xml",
                r#"<Unknown xmlns="urn:unknown" version="2.20"/>"#,
                "unsupported",
            ),
            (
                "unknown-versionless",
                "Unknown.xml",
                r#"<Unknown xmlns="urn:unknown"/>"#,
                "unsupported",
            ),
            (
                "versionless-family-with-version",
                "Templates/Main/Ext/Template.xml",
                r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" version="2.20"/>"#,
                "versionless",
            ),
        ];
        for (case, relative, xml, expected) in cases {
            let root = std::env::temp_dir().join(format!(
                "unica-staged-root-registry-{case}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, xml).unwrap();

            let error = validate_staged_dump(&root, SourceSetKind::Configuration)
                .expect_err("unregistered or inexact staged XML must fail closed");

            assert!(error.contains(expected), "{case}: {error}");
            std::fs::remove_dir_all(root).unwrap();
        }

        let root = std::env::temp_dir().join(format!(
            "unica-staged-root-registry-versionless-control-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("Templates/Main/Ext")).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
        std::fs::write(
            root.join("Templates/Main/Ext/Template.xml"),
            r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
        )
        .unwrap();
        validate_staged_dump(&root, SourceSetKind::Configuration)
            .expect("registered versionless subordinate family remains allowed");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// One staged file and independently recorded version policy per
    /// `STAGED_ROOT_REGISTRY` entry, at the dump-relative path the platform
    /// writes it to.
    ///
    /// Where a checked-in fixture exists the case uses those bytes, keeping the
    /// platform BOM and CRLF. The remaining cases are the smallest document that
    /// still carries the registered root, because the guard reads nothing below
    /// it. `staged_root_registry_covers_every_registered_root` fails if a
    /// registry entry has no case here.
    const STAGED_ROOT_CASES: &[(&str, &[u8], StagedRootVersionPolicy)] = &[
        (
            "Configuration.xml",
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties/></Configuration></MetaDataObject>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Catalogs/Товары/Forms/Форма/Ext/Form.xml",
            br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Subsystems/Продажи/Ext/CommandInterface.xml",
            br#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Catalogs/Товары/Ext/Help.xml",
            br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "ExchangePlans/Обмен/Ext/Content.xml",
            include_bytes!("../../../../tests/fixtures/platform_8_3_27/exchange_plan/Content.xml"),
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Ext/HomePageWorkArea.xml",
            br#"<HomePageWorkArea xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "CommonPictures/Логотип/Ext/Picture.xml",
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/Picture.xml"
            ),
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "ScheduledJobs/ОбновлениеКурсов/Ext/Schedule.xml",
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/Schedule.xml"
            ),
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Catalogs/Товары/Ext/Predefined.xml",
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/Predefined.xml"
            ),
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "ConfigDumpInfo.xml",
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/ConfigDumpInfo.xml"
            ),
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "BusinessProcesses/Согласование/Ext/Flowchart.xml",
            br#"<GraphicalSchema xmlns="http://v8.1c.ru/8.3/xcf/scheme" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Roles/Администратор/Ext/Rights.xml",
            br#"<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Reports/Продажи/Templates/Основной/Ext/Template.xml",
            br#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
            StagedRootVersionPolicy::Versionless,
        ),
        (
            "CommonTemplates/ОформлениеОтчетов/Ext/Template.xml",
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/Template.xml"
            ),
            StagedRootVersionPolicy::Versionless,
        ),
        (
            "CommonTemplates/ПечатнаяФорма/Ext/Template.xml",
            include_bytes!("../../../../tests/fixtures/platform_8_3_27/mxl/Template.xml"),
            StagedRootVersionPolicy::Versionless,
        ),
        (
            "WSReferences/СлужбаОбмена/Ext/WSDefinition.xml",
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/WSDefinition.xml"
            ),
            StagedRootVersionPolicy::Versionless,
        ),
        (
            "Ext/ClientApplicationInterface.xml",
            br#"<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core"/>"#,
            // Publication syntax is versionless; the independent owner policy
            // records that this document belongs to its containing source set.
            StagedRootVersionPolicy::Versionless,
        ),
    ];

    fn staged_root_case_root(source: &[u8]) -> (String, String) {
        let text = std::str::from_utf8(source).expect("staged root case is UTF-8");
        let document = Document::parse(text.trim_start_matches('\u{feff}'))
            .expect("staged root case parses as XML");
        let root = document.root_element();
        (
            root.tag_name().namespace().unwrap_or_default().to_string(),
            root.tag_name().name().to_string(),
        )
    }

    fn staged_root_case_tree(case: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-staged-root-case-{case}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
        root
    }

    #[test]
    fn staged_root_registry_covers_every_registered_root() {
        let covered = STAGED_ROOT_CASES
            .iter()
            .map(|(_, source, _)| staged_root_case_root(source))
            .collect::<BTreeSet<_>>();
        let uncovered = STAGED_ROOT_REGISTRY
            .iter()
            .filter(|root| {
                !covered.contains(&(root.namespace.to_string(), root.local_name.to_string()))
            })
            .map(|root| format!("{{{}}}{}", root.namespace, root.local_name))
            .collect::<Vec<_>>();

        assert!(
            uncovered.is_empty(),
            "every registered staged root needs a case: {uncovered:?}"
        );
    }

    #[test]
    fn staged_root_registry_publishes_every_platform_8_3_27_dump_root() {
        let root = staged_root_case_tree("accepted");
        for (relative, source, _) in STAGED_ROOT_CASES {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, source).unwrap();
        }

        validate_staged_dump(&root, SourceSetKind::Configuration).unwrap_or_else(|error| {
            panic!("every platform-written dump root must stay publishable: {error}")
        });

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_root_registry_binds_each_platform_root_to_its_version_policy() {
        for (index, (relative, source, expected_policy)) in STAGED_ROOT_CASES.iter().enumerate() {
            let (namespace, local_name) = staged_root_case_root(source);
            let policy = staged_root_version_policy(&namespace, &local_name)
                .unwrap_or_else(|| panic!("{relative}: case root must be registered"));
            assert_eq!(
                policy, *expected_policy,
                "{relative}: registry policy must match the independently recorded platform evidence"
            );
            let text = std::str::from_utf8(source).unwrap();
            let source_without_bom = text.trim_start_matches('\u{feff}');
            let document = Document::parse(source_without_bom).unwrap();
            let root_node = document.root_element();
            let root_start = text.len() - source_without_bom.len() + root_node.range().start;
            let root_tag_end = root_start
                + text[root_start..]
                    .find('>')
                    .expect("case root opening tag is well formed");
            let root_tag = &text[root_start..root_tag_end];
            let (mutated, expected) = match expected_policy {
                StagedRootVersionPolicy::ExactRootVersion => {
                    assert_eq!(
                        root_node.attribute("version"),
                        Some("2.20"),
                        "{relative}: the root must carry the exact 2.20 value"
                    );
                    let version_literal = r#"version="2.20""#;
                    let version_start = root_start
                        + root_tag.find(version_literal).unwrap_or_else(|| {
                            panic!("{relative}: root must carry the raw 2.20 literal")
                        });
                    let mut mutated = text.to_string();
                    mutated.replace_range(
                        version_start..version_start + version_literal.len(),
                        r#"version="2.19""#,
                    );
                    (mutated, "2.19")
                }
                StagedRootVersionPolicy::Versionless => {
                    assert!(
                        root_node.attribute("version").is_none(),
                        "{relative}: the root must stay versionless"
                    );
                    let tag_name_end = root_start
                        + text[root_start..]
                            .find(|character: char| {
                                character.is_whitespace() || character == '/' || character == '>'
                            })
                            .expect("case root element is well formed");
                    let mut mutated = text.to_string();
                    mutated.insert_str(tag_name_end, r#" version="2.20""#);
                    (mutated, "versionless")
                }
            };
            let root = staged_root_case_tree(&index.to_string());
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, mutated).unwrap();

            let error = validate_staged_dump(&root, SourceSetKind::Configuration)
                .expect_err("a registered root with the wrong version policy must fail closed");

            assert!(error.contains(expected), "{relative}: {error}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn staged_xml_bytes_must_match_the_tree_snapshot_even_if_path_is_restored() {
        let root =
            std::env::temp_dir().join(format!("unica-staged-read-race-{}", uuid::Uuid::new_v4()));
        let form = root.join("Forms/Main/Ext/Form.xml");
        std::fs::create_dir_all(form.parent().unwrap()).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
        let invalid =
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.19"/>"#.to_string();
        let valid = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#.to_string();
        std::fs::write(&form, &invalid).unwrap();
        let form_before = form.clone();
        let form_after = form.clone();

        let result = with_targeted_read_hooks(
            form.clone(),
            move || std::fs::write(&form_before, valid).unwrap(),
            move || std::fs::write(&form_after, invalid).unwrap(),
            || validate_staged_dump(&root, SourceSetKind::Configuration),
        );

        let error = result.expect_err(
            "XML bytes parsed during validation must be bound to the captured tree snapshot",
        );
        assert!(
            error.contains("snapshot") || error.contains("changed"),
            "{error}"
        );
        assert!(
            std::fs::read_to_string(&form).unwrap().contains("2.19"),
            "the test must restore the invalid preimage before validation returns"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_swap_between_file_inspection_and_open_is_rejected() {
        let root =
            std::env::temp_dir().join(format!("unica-staged-file-swap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let owner = root.join("Configuration.xml");
        let original = root.join("Configuration.original.xml");
        let alternate = root.join("Configuration.alternate.xml");
        std::fs::write(&owner, valid_configuration_owner()).unwrap();
        std::fs::write(&alternate, valid_configuration_owner()).unwrap();
        let owner_for_hook = owner.clone();
        let original_for_hook = original.clone();
        let alternate_for_hook = alternate.clone();

        let result = with_secure_read_hook(
            move |path| {
                assert_eq!(path, owner_for_hook);
                std::fs::rename(&owner_for_hook, &original_for_hook).unwrap();
                std::os::unix::fs::symlink(&alternate_for_hook, &owner_for_hook).unwrap();
            },
            || super::secure_read_regular_file(&owner, "swap fixture"),
        );

        let error = result.expect_err("O_NOFOLLOW/identity binding must reject the swap");
        assert!(
            error.contains("symbolic link")
                || error.contains("changed")
                || error.contains("identity"),
            "{error}"
        );
        std::fs::remove_file(&owner).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_swap_between_directory_inspection_and_open_is_rejected() {
        let root = std::env::temp_dir().join(format!(
            "unica-staged-directory-swap-{}",
            uuid::Uuid::new_v4()
        ));
        let target = root.join("target");
        let original = root.join("target-original");
        let outside = root.join("outside");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(target.join("inside"), b"inside").unwrap();
        std::fs::write(outside.join("outside"), b"outside").unwrap();
        let target_for_hook = target.clone();
        let original_for_hook = original.clone();
        let outside_for_hook = outside.clone();

        let result = with_tree_open_hook(
            move |path| {
                assert_eq!(path, target_for_hook);
                std::fs::rename(&target_for_hook, &original_for_hook).unwrap();
                std::os::unix::fs::symlink(&outside_for_hook, &target_for_hook).unwrap();
            },
            || TreeSnapshot::capture_target(&target),
        );

        let error = result.expect_err("directory descriptor open must reject the swap");
        assert!(
            error.contains("symbolic link")
                || error.contains("changed")
                || error.contains("identity"),
            "{error}"
        );
        std::fs::remove_file(&target).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    fn canonical_publication(
        context: &WorkspaceContext,
        config: &Path,
        store_path: &Path,
        runner: &dyn ProcessRunner,
    ) -> super::VerifiedFullDumpPublication {
        let cancellation = CancellationToken::new();
        let store = super::RunnerProjectStore::open_and_lock(store_path, &cancellation).unwrap();
        super::prepare_publication(
            context,
            super::FullDumpSelection {
                config_path: config,
                source_set: Some("main"),
                extension: None,
                expected_target: None,
            },
            store,
            runner,
            &cancellation,
            |mut primary, local, _| {
                if let Some(local) = local {
                    super::merge_yaml_values(&mut primary, local);
                }
                Ok(primary)
            },
            |effective, _| Ok((effective, None)),
            crate::infrastructure::runtime_composition::full_dump_semantics(),
        )
        .unwrap()
    }

    fn fill_canonical_stage(publication: &mut super::VerifiedFullDumpPublication) {
        use sha2::{Digest, Sha256};
        use std::io::Write;

        assert!(
            publication.staged_target().is_dir(),
            "canonical preparation must create the held stage"
        );
        let stage = publication.staged_target().to_path_buf();
        let owner = valid_configuration_owner();
        let mut entries = Vec::new();
        for (name, bytes) in [
            ("Configuration.xml", owner.as_bytes()),
            ("payload.bsl", b"validated source".as_slice()),
        ] {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(stage.join(name))
                .unwrap();
            file.write_all(bytes).unwrap();
            entries.push(super::TreeEntrySnapshot {
                relative_path: PathBuf::from(name),
                kind: super::TreeEntryKind::File {
                    identity: super::file_identity(&file).unwrap(),
                    size: bytes.len() as u64,
                    sha256: Sha256::digest(bytes).into(),
                },
            });
        }
        publication.test_remember_staged_fixture(&TreeSnapshot::Directory {
            identity: publication
                .test_observation()
                .canonical_stage_identity
                .unwrap(),
            entries,
        });
    }

    fn attest_canonical(
        publication: &mut super::VerifiedFullDumpPublication,
        platform: &FixedPlatform,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
    ) {
        super::with_test_platform(&platform.executable, &platform.version, || {
            super::attest_selected_local(publication, &platform.executable, runner, cancellation)
                .unwrap()
        });
    }

    #[test]
    fn canonical_preparation_creates_an_empty_owner_only_held_stage_before_dispatch() {
        let (root, context, config) = workspace("canonical-created-empty-stage");
        let runner = DumpRunner::valid();
        let publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let stage = publication.staged_target().to_path_buf();
        let private_root = publication.test_observation().private_root;
        assert!(stage.is_dir());
        assert_eq!(std::fs::read_dir(&stage).unwrap().count(), 0);
        publication.test_assert_held_stage_binding();
        publication.recheck_before_dispatch().unwrap();
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(!context.cwd.join("src").exists());
        publication.abort().unwrap();
        assert!(
            !private_root.exists(),
            "terminal cleanup must release the created child handle on Windows too"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn verify_canonical_stage_handoff_replacement_cleanup(explicit_abort: bool) {
        let (root, context, config) = workspace("canonical-stage-handoff-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let original_target = TreeSnapshot::capture_target(&target).unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let owned_identity = publication
            .test_observation()
            .canonical_stage_identity
            .unwrap();
        let stage = publication.staged_target().to_path_buf();
        let displaced =
            super::normalize_path_identity(&context.cwd.join("owned-stage-displaced-at-handoff"))
                .unwrap();
        let hook_stage = stage.clone();
        let hook_displaced = displaced.clone();
        let publication_result = with_publication_hook(
            PublicationCheckpoint::BeforeStageInstall,
            move || {
                std::fs::rename(&hook_stage, &hook_displaced).unwrap();
                std::fs::create_dir(&hook_stage).unwrap();
                std::fs::write(
                    hook_stage.join("foreign.txt"),
                    b"foreign bytes after handle release",
                )
                .unwrap();
            },
            || publication.publish(&cancellation),
        );
        assert!(
            publication_result.is_err(),
            "seal must refuse the replaced stage root"
        );
        assert!(!cancellation.protected_process_started());
        let foreign_identity =
            super::file_identity(&super::open_directory_nofollow(&stage).unwrap()).unwrap();
        assert_ne!(foreign_identity, owned_identity);
        let cleanup = if explicit_abort {
            Some(publication.abort())
        } else {
            drop(publication);
            None
        };
        assert_eq!(
            std::fs::read(stage.join("foreign.txt")).unwrap(),
            b"foreign bytes after handle release",
            "terminal cleanup must preserve the foreign stage after a failed handoff"
        );
        assert_eq!(
            super::file_identity(&super::open_directory_nofollow(&stage).unwrap()).unwrap(),
            foreign_identity
        );
        assert_eq!(
            super::file_identity(&super::open_directory_nofollow(&displaced).unwrap()).unwrap(),
            owned_identity
        );
        assert_eq!(
            std::fs::read(displaced.join("payload.bsl")).unwrap(),
            b"validated source"
        );
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            original_target
        );
        if let Some(cleanup) = cleanup {
            let failure = cleanup.unwrap_err();
            assert!(matches!(failure.phase, super::FullDumpPhase::Integrity));
            assert!(matches!(&failure.recovery, Ok(Some(_))));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_stage_handoff_replacement_survives_explicit_abort() {
        verify_canonical_stage_handoff_replacement_cleanup(true);
    }

    #[test]
    fn canonical_stage_handoff_replacement_survives_unexpected_drop() {
        verify_canonical_stage_handoff_replacement_cleanup(false);
    }

    #[cfg(unix)]
    fn verify_canonical_stage_cleanup_ancestor_replacement(explicit_abort: bool) {
        let (root, context, config) = workspace("canonical-stage-cleanup-ancestor-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let original_target = TreeSnapshot::capture_target(&target).unwrap();
        let runner = DumpRunner::valid();
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let private_root = publication.test_observation().private_root;
        let execution = publication.test_observation().execution;
        let owned_stage_identity = publication
            .test_observation()
            .canonical_stage_identity
            .unwrap();
        let owned_execution_identity = publication.test_observation().execution_identity;
        let displaced = private_root.join("displaced-execution");
        std::fs::rename(&execution, &displaced).unwrap();
        std::fs::create_dir(&execution).unwrap();
        std::fs::write(
            execution.join("foreign.txt"),
            b"foreign execution replacement sentinel",
        )
        .unwrap();
        let foreign_execution_identity =
            super::file_identity(&super::open_directory_nofollow(&execution).unwrap()).unwrap();
        assert_ne!(foreign_execution_identity, owned_execution_identity);
        let cleanup = if explicit_abort {
            Some(publication.abort())
        } else {
            drop(publication);
            None
        };
        assert_eq!(
            std::fs::read(execution.join("foreign.txt")).unwrap(),
            b"foreign execution replacement sentinel",
            "cleanup must refuse a replaced execution ancestor before reading its held child"
        );
        assert_eq!(
            super::file_identity(&super::open_directory_nofollow(&execution).unwrap()).unwrap(),
            foreign_execution_identity
        );
        assert_eq!(
            super::file_identity(&super::open_directory_nofollow(&displaced).unwrap()).unwrap(),
            owned_execution_identity
        );
        assert_eq!(
            super::file_identity(
                &super::open_directory_nofollow(&displaced.join("staged-source")).unwrap()
            )
            .unwrap(),
            owned_stage_identity
        );
        assert_eq!(
            std::fs::read(displaced.join("staged-source/payload.bsl")).unwrap(),
            b"validated source"
        );
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            original_target
        );
        if let Some(cleanup) = cleanup {
            let failure = cleanup.unwrap_err();
            assert!(matches!(failure.phase, super::FullDumpPhase::Integrity));
            assert_eq!(failure.recovery, Ok(Some(private_root)));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_stage_cleanup_ancestor_replacement_survives_explicit_abort() {
        verify_canonical_stage_cleanup_ancestor_replacement(true);
    }

    #[cfg(unix)]
    #[test]
    fn canonical_stage_cleanup_ancestor_replacement_survives_unexpected_drop() {
        verify_canonical_stage_cleanup_ancestor_replacement(false);
    }

    #[test]
    fn canonical_stage_replacement_is_refused_and_foreign_tree_survives_abort() {
        let (root, context, config) = workspace("canonical-held-stage-replacement");
        seed_valid_target(&context.cwd.join("src"));
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let stage = publication.staged_target().to_path_buf();
        let private_root = publication.test_observation().private_root;
        let displaced = context.cwd.join("retained-owned-stage");
        std::fs::rename(&stage, &displaced).unwrap();
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("foreign.txt"), b"foreign stage replacement").unwrap();
        assert!(matches!(
            publication.recheck_before_dispatch().unwrap_err().phase,
            super::FullDumpPhase::Integrity
        ));
        assert!(matches!(
            publication.publish(&cancellation).unwrap_err().phase,
            super::FullDumpPhase::Integrity
        ));
        assert!(!cancellation.protected_process_started());
        assert_eq!(
            std::fs::read(context.cwd.join("src/before.txt")).unwrap(),
            b"before"
        );
        let cleanup = publication.abort().unwrap_err();
        assert!(matches!(cleanup.phase, super::FullDumpPhase::Integrity));
        assert_eq!(cleanup.recovery, Ok(Some(private_root)));
        assert_eq!(
            std::fs::read(stage.join("foreign.txt")).unwrap(),
            b"foreign stage replacement"
        );
        assert_eq!(
            std::fs::read(displaced.join("payload.bsl")).unwrap(),
            b"validated source"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_stage_replacement_also_survives_an_unexpected_guard_drop() {
        let (root, context, config) = workspace("canonical-held-stage-drop");
        let runner = DumpRunner::valid();
        let publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let stage = publication.staged_target().to_path_buf();
        let displaced = context.cwd.join("retained-owned-empty-stage");
        std::fs::rename(&stage, &displaced).unwrap();
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("foreign.txt"), b"preserved on implicit drop").unwrap();
        drop(publication);
        assert_eq!(
            std::fs::read(stage.join("foreign.txt")).unwrap(),
            b"preserved on implicit drop"
        );
        assert!(displaced.is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_preparation_keeps_the_runner_owned_stage_creation_behavior() {
        let (root, context, _) = workspace("legacy-stage-remains-runner-owned");
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut prepared = super::prepare_legacy(
            &args(),
            &context,
            &platform,
            &runner,
            &CancellationToken::new(),
            crate::infrastructure::runtime_composition::full_dump_semantics(),
        )
        .unwrap();
        assert!(!prepared.test_has_canonical_stage());
        assert!(!prepared.staged_target().exists());
        assert!(prepared.cleanup_observation().cleanup_error.is_none());
        drop(prepared);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_preparation_keeps_logical_target_and_attests_only_the_selected_executable() {
        let (root, context, config) = workspace("canonical-selected-provider");
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        assert_eq!(
            runner.calls.load(Ordering::SeqCst),
            0,
            "preparation must not probe a provider"
        );
        let effective: serde_yaml::Value =
            serde_yaml::from_slice(&std::fs::read(publication.effective_config()).unwrap())
                .unwrap();
        assert_eq!(
            effective["source-set"][0]["path"].as_str(),
            Some(publication.target().to_str().unwrap())
        );
        assert!(
            effective.get("tools").is_none(),
            "selection must not be forced through tools.platform"
        );
        fill_canonical_stage(&mut publication);
        assert!(
            publication.publish(&CancellationToken::new()).is_err(),
            "an unattested provider cannot publish"
        );
        super::with_test_platform(&platform.executable, &platform.version, || {
            assert!(super::attest_selected_local(
                &mut publication,
                &root.join("foreign/1cv8"),
                &runner,
                &CancellationToken::new()
            )
            .is_err());
        });
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(publication.selected_extension(), None);
        publication.abort().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_nested_target_prepares_parents_after_selected_and_retains_store_until_finish() {
        let (root, context, config) = workspace("canonical-nested-target");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let store_path = root.join("projection");
        let mut publication = canonical_publication(&context, &config, &store_path, &runner);
        let ancestor = super::normalize_path_identity(&context.cwd).unwrap();
        assert!(publication.staged_target().starts_with(&ancestor));
        assert!(!context.cwd.join("newsrc").exists());
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        assert!(context.cwd.join("newsrc/nested").is_dir());
        assert!(
            !cancellation.protected_process_started(),
            "dispatch parent preparation is reversible"
        );
        let retained_parent = publication
            .test_observation()
            .opened_target_parent_identity
            .unwrap();
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        assert_eq!(
            publication.test_observation().created_target_parents,
            2,
            "repeated preparation must keep one ledger"
        );
        assert_eq!(
            publication
                .test_observation()
                .opened_target_parent_identity
                .unwrap(),
            retained_parent
        );
        let result = publication.publish(&cancellation).unwrap();
        assert!(!result.replaced, "the retained initial target was absent");
        assert_eq!(result.files, 2);
        assert!(
            publication
                .test_observation()
                .canonical_stage_identity
                .is_none(),
            "the shared publisher takes stage ownership after Ready validation"
        );
        assert!(publication.is_published());
        assert!(cancellation.protected_process_started());
        cancellation.cancel();
        assert_eq!(
            publication.publish(&cancellation).unwrap().files,
            2,
            "publication fact survives cancellation and repeated calls"
        );
        assert_eq!(
            std::fs::read(context.cwd.join("newsrc/nested/Cf/payload.bsl")).unwrap(),
            b"validated source"
        );
        let waiting = CancellationToken::new();
        let thread_cancel = waiting.clone();
        let waiting_path = store_path.clone();
        let (contended_tx, contended_rx) = std::sync::mpsc::channel();
        let contender = std::thread::spawn(move || {
            super::with_test_project_lock_contention(
                move || contended_tx.send(()).unwrap(),
                || super::RunnerProjectStore::open_and_lock(&waiting_path, &thread_cancel).err(),
            )
        });
        contended_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        waiting.cancel();
        assert!(contender.join().unwrap().is_some());
        publication.finish().unwrap();
        let acquired =
            super::RunnerProjectStore::open_and_lock(&store_path, &CancellationToken::new())
                .unwrap();
        drop(acquired);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_selected_parent_preparation_aborts_only_its_own_empty_ancestry() {
        let (root, context, config) = workspace("canonical-selected-parent-abort");
        std::fs::create_dir(context.cwd.join("newsrc")).unwrap();
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/deeper/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        assert!(
            !context.cwd.join("newsrc/nested").exists(),
            "preparation before Selected stays write-free for source ancestry"
        );
        let cancellation = CancellationToken::new();
        assert!(
            publication.prepare_dispatch_parent(&cancellation).is_err(),
            "unattested Selected cannot create source ancestry"
        );
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        assert!(context.cwd.join("newsrc/nested/deeper").is_dir());
        assert!(!publication.target().exists());
        assert!(!cancellation.protected_process_started());
        publication.abort().unwrap();
        assert!(
            context.cwd.join("newsrc").is_dir(),
            "pre-existing parent must survive"
        );
        assert!(!context.cwd.join("newsrc/nested").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_selected_parent_abort_preserves_a_late_foreign_file_with_typed_recovery() {
        let (root, context, config) = workspace("canonical-selected-parent-foreign-file");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let foreign = context.cwd.join("newsrc/nested/foreign.txt");
        std::fs::write(&foreign, b"created by another owner after Selected").unwrap();
        let failure = publication.abort().unwrap_err();
        assert!(matches!(failure.phase, super::FullDumpPhase::Cleanup));
        assert!(matches!(&failure.recovery, Ok(Some(_))));
        assert_eq!(
            std::fs::read(&foreign).unwrap(),
            b"created by another owner after Selected"
        );
        assert!(!cancellation.protected_process_started());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_selected_parent_abort_preserves_a_replaced_parent_identity() {
        let (root, context, config) = workspace("canonical-selected-parent-identity-swap");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let parent = context.cwd.join("newsrc/nested");
        let displaced = context.cwd.join("newsrc/displaced-nested");
        std::fs::rename(&parent, &displaced).unwrap();
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("foreign.txt"), b"foreign parent replacement").unwrap();
        let failure = publication.abort().unwrap_err();
        assert!(matches!(&failure.recovery, Ok(Some(_))));
        assert_eq!(
            std::fs::read(parent.join("foreign.txt")).unwrap(),
            b"foreign parent replacement"
        );
        assert!(
            displaced.is_dir(),
            "the original retained identity must also survive"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_cancelled_selected_parent_preparation_has_a_causal_typed_refusal() {
        let (root, context, config) = workspace("canonical-selected-parent-cancel");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        cancellation.cancel();
        let failure = publication
            .prepare_dispatch_parent(&cancellation)
            .unwrap_err();
        assert!(matches!(failure.phase, super::FullDumpPhase::Cancelled));
        assert!(!context.cwd.join("newsrc").exists());
        assert!(!cancellation.protected_process_started());
        publication.abort().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_publish_never_recreates_unprepared_dispatch_parents() {
        let (root, context, config) = workspace("canonical-unprepared-dispatch-parent");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        assert!(publication.publish(&cancellation).is_err());
        assert!(!context.cwd.join("newsrc").exists());
        assert!(!cancellation.protected_process_started());
        publication.abort().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_cancel_before_commit_leaves_missing_source_parents_absent() {
        let (root, context, config) = workspace("canonical-cancel-missing-parent");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let hook_cancellation = cancellation.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::BeforeStageInstall,
            move || hook_cancellation.cancel(),
            || publication.publish(&cancellation),
        );
        assert!(matches!(
            result.unwrap_err().phase,
            super::FullDumpPhase::Cancelled
        ));
        assert!(!publication.is_published());
        assert!(!cancellation.protected_process_started());
        assert!(
            context.cwd.join("newsrc/nested").is_dir(),
            "dispatch ancestry stays held until terminal cleanup"
        );
        publication.abort().unwrap();
        assert!(!context.cwd.join("newsrc").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_late_cancellation_does_not_interrupt_the_shared_publication_transaction() {
        let (root, context, config) = workspace("canonical-late-cancel");
        seed_valid_target(&context.cwd.join("src"));
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let hook_cancellation = cancellation.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::AfterBackup,
            move || {
                assert!(
                    hook_cancellation.protected_process_started(),
                    "first namespace effect must be sticky before scanning/commit"
                );
                hook_cancellation.cancel();
            },
            || publication.publish(&cancellation),
        );
        assert!(
            result.is_ok(),
            "late cancellation must allow the held transaction to finish"
        );
        assert!(
            result.unwrap().replaced,
            "the retained initial tree was replaced"
        );
        assert!(publication.is_published());
        assert!(cancellation.is_cancelled());
        assert_eq!(
            std::fs::read(context.cwd.join("src/payload.bsl")).unwrap(),
            b"validated source"
        );
        publication.finish().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_shared_publisher_keeps_post_rename_content_checks_and_rolls_back_mutation() {
        let (root, context, config) = workspace("canonical-post-rename-content-change");
        seed_valid_target(&context.cwd.join("src"));
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let hook_target = context.cwd.join("src/payload.bsl");
        let hook_cancellation = cancellation.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::write(hook_target, b"unvalidated same-inode content").unwrap();
                hook_cancellation.cancel();
            },
            || publication.publish(&cancellation),
        );
        assert!(
            matches!(result.unwrap_err().phase, super::FullDumpPhase::Publication),
            "post-rename mutation stays a publication failure even if cancellation arrives later"
        );
        assert!(!publication.is_published());
        assert!(
            cancellation.protected_process_started(),
            "an attempted public effect stays factual even after rollback"
        );
        assert_eq!(
            std::fs::read(context.cwd.join("src/before.txt")).unwrap(),
            b"before"
        );
        publication.abort().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_missing_parent_refuses_a_foreign_directory_without_adopting_it() {
        let (root, context, config) = workspace("canonical-foreign-parent");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        std::fs::create_dir(context.cwd.join("newsrc")).unwrap();
        std::fs::write(context.cwd.join("newsrc/foreign.txt"), b"foreign parent").unwrap();
        assert!(publication.prepare_dispatch_parent(&cancellation).is_err());
        assert!(publication.publish(&cancellation).is_err());
        assert_eq!(
            std::fs::read(context.cwd.join("newsrc/foreign.txt")).unwrap(),
            b"foreign parent"
        );
        assert!(!context.cwd.join("newsrc/nested").exists());
        publication.abort().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_failed_commit_removes_only_its_empty_missing_parents() {
        let (root, context, config) = workspace("canonical-parent-rollback");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let result =
            with_publication_failpoint(PublicationCheckpoint::BeforeSealedPublishRename, || {
                publication.publish(&cancellation)
            });
        assert!(result.is_err());
        assert!(context.cwd.join("newsrc/nested").is_dir());
        publication.abort().unwrap();
        assert!(!context.cwd.join("newsrc").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_parent_rollback_preserves_foreign_children_in_its_created_chain() {
        let (root, context, config) = workspace("canonical-parent-foreign-child");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let foreign = context.cwd.join("newsrc/nested/foreign.txt");
        let hook_foreign = foreign.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::BeforeSealedPublishRename,
            move || {
                std::fs::write(&hook_foreign, b"concurrent source sibling").unwrap();
            },
            || {
                with_publication_failpoint(PublicationCheckpoint::BeforeSealedPublishRename, || {
                    publication.publish(&cancellation)
                })
            },
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&foreign).unwrap(),
            b"concurrent source sibling"
        );
        assert!(!publication.target().exists());
        let cleanup = publication.abort().unwrap_err();
        assert!(
            matches!(&cleanup.recovery, Ok(Some(_))),
            "foreign sibling must retain a typed recovery boundary"
        );
        assert_eq!(
            std::fs::read(&foreign).unwrap(),
            b"concurrent source sibling"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_cleanup_failure_after_publication_is_a_warning_with_changed_sources() {
        let (root, context, config) = workspace("canonical-published-cleanup-warning");
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        publication.publish(&cancellation).unwrap();
        let public_before_cleanup =
            super::capture_tree_target_nofollow(&context.cwd.join("src")).unwrap();
        let private = publication.test_observation().private_root;
        let owned_before = super::capture_tree_target_nofollow(&private).unwrap();
        let displaced = context.cwd.join("displaced-private");
        std::fs::rename(&private, &displaced).unwrap();
        std::fs::create_dir(&private).unwrap();
        std::fs::write(private.join("foreign.txt"), b"preserve foreign root").unwrap();
        let foreign_before = super::capture_tree_target_nofollow(&private).unwrap();
        let finish = publication
            .finish()
            .expect("published cleanup failure must not become unchanged failure");
        assert_eq!(finish.warnings.len(), 1);
        let location_note = finish
            .retained
            .expect_err("a foreign replacement is not the retained private directory");
        assert!(location_note.contains("current location is unconfirmed"));
        assert!(location_note.contains(&format!("original name: {}", private.display())));
        assert_eq!(
            super::capture_tree_target_nofollow(&displaced).unwrap(),
            owned_before,
            "finalization must preserve every owned private identity and byte"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&private).unwrap(),
            foreign_before,
            "finalization must preserve every foreign identity and byte"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&context.cwd.join("src")).unwrap(),
            public_before_cleanup,
            "a private reporting failure must preserve the verified publication"
        );
        assert_eq!(
            std::fs::read(context.cwd.join("src/payload.bsl")).unwrap(),
            b"validated source"
        );
        assert_eq!(
            std::fs::read(private.join("foreign.txt")).unwrap(),
            b"preserve foreign root"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn projection_values(password: &str) -> (serde_yaml::Value, serde_yaml::Value) {
        let primary =
            serde_yaml::from_str("workPath: .work\nformat: DESIGNER\nsource-set: []\n").unwrap();
        let local = serde_yaml::from_str(&format!(
            "infobases:\n  origin:\n    connection: 'File=/retained/owned/base'\n    password: '{password}'\n")).unwrap();
        (primary, local)
    }

    fn read_projection_primary(path: &Path) -> serde_yaml::Value {
        serde_yaml::from_slice(&std::fs::read(path.join(super::EFFECTIVE_CONFIG_NAME)).unwrap())
            .unwrap()
    }

    #[test]
    fn runner_project_projection_keeps_a_null_primary_through_the_local_gap() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        write_projection(&mut store, &primary, Some(&before)).unwrap();
        let observed = std::rc::Rc::new(std::cell::Cell::new(false));
        let hook_observed = observed.clone();
        super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::LocalGap,
            move |path| {
                assert!(
                    !path.join(super::LOCAL_CONFIG_NAME).exists(),
                    "the hook must observe the real displacement gap"
                );
                assert!(
                    read_projection_primary(path).is_null(),
                    "owner guard must be unreadable while the local layer is absent"
                );
                let guard = std::fs::File::open(path.join(super::EFFECTIVE_CONFIG_NAME)).unwrap();
                super::verify_private_owner(&guard).unwrap();
                hook_observed.set(true);
            },
            || write_projection(&mut store, &primary, Some(&after)),
        )
        .unwrap();
        assert!(observed.get());
        assert_eq!(read_projection_primary(&path), primary);
        let projected: serde_yaml::Value =
            serde_yaml::from_slice(&std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap())
                .unwrap();
        assert_eq!(projected, after);
        store.verify().unwrap();
    }

    #[test]
    fn runner_project_projection_removal_keeps_the_guard_until_local_absence_is_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, local) = projection_values("before");
        write_projection(&mut store, &primary, Some(&local)).unwrap();
        let observed = std::rc::Rc::new(std::cell::Cell::new(false));
        let hook_observed = observed.clone();
        super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::LocalGap,
            move |path| {
                assert!(!path.join(super::LOCAL_CONFIG_NAME).exists());
                assert!(read_projection_primary(path).is_null());
                hook_observed.set(true);
            },
            || write_projection(&mut store, &primary, None),
        )
        .unwrap();
        assert!(observed.get());
        assert_eq!(
            read_projection_primary(&path),
            primary,
            "an intentional completed removal must restore the valid primary"
        );
        assert!(!path.join(super::LOCAL_CONFIG_NAME).exists());
        store.verify().unwrap();
    }

    #[test]
    fn runner_project_projection_failure_after_guard_stays_unreadable_until_next_owned_repair() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        write_projection(&mut store, &primary, Some(&before)).unwrap();
        let stable_config = store.config_path().to_path_buf();
        let previous_local = std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap();
        let result = super::with_runner_projection_failpoint(
            super::RunnerProjectionCheckpoint::GuardInstalled,
            || write_projection(&mut store, &primary, Some(&after)),
        );
        assert!(result.is_err());
        assert!(read_projection_primary(&path).is_null());
        assert_eq!(
            std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap(),
            previous_local,
            "failure must not delete the old local layer"
        );
        store.verify().unwrap();
        write_projection(&mut store, &primary, Some(&after)).unwrap();
        assert_eq!(store.config_path(), stable_config);
        assert_eq!(read_projection_primary(&path), primary);
        let repaired: serde_yaml::Value =
            serde_yaml::from_slice(&std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap())
                .unwrap();
        assert_eq!(repaired, after);
        assert_eq!(
            repaired["infobases"]["origin"]["connection"],
            before["infobases"]["origin"]["connection"],
            "repair keeps the same source/infobase ownership recipe"
        );
    }

    #[test]
    fn runner_project_projection_error_in_local_gap_retains_guard_and_restores_old_owned_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        write_projection(&mut store, &primary, Some(&before)).unwrap();
        let previous_local = std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap();
        let result = super::with_runner_projection_failpoint(
            super::RunnerProjectionCheckpoint::LocalGap,
            || write_projection(&mut store, &primary, Some(&after)),
        );
        assert!(result.is_err());
        assert!(read_projection_primary(&path).is_null());
        assert_eq!(
            std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap(),
            previous_local
        );
        store.verify().unwrap();
        write_projection(&mut store, &primary, Some(&after)).unwrap();
        assert_eq!(read_projection_primary(&path), primary);
    }

    #[test]
    fn runner_project_projection_error_after_local_confirmation_keeps_guard_until_repair() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        write_projection(&mut store, &primary, Some(&before)).unwrap();
        let result = super::with_runner_projection_failpoint(
            super::RunnerProjectionCheckpoint::LocalConfirmed,
            || write_projection(&mut store, &primary, Some(&after)),
        );
        assert!(result.is_err());
        assert!(read_projection_primary(&path).is_null());
        assert_eq!(
            serde_yaml::from_slice::<serde_yaml::Value>(
                &std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap()
            )
            .unwrap(),
            after
        );
        store.verify().unwrap();
        write_projection(&mut store, &primary, Some(&after)).unwrap();
        assert_eq!(read_projection_primary(&path), primary);
    }

    #[test]
    fn runner_project_projection_preserves_a_foreign_primary_swapped_after_guard_installation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        write_projection(&mut store, &primary, Some(&before)).unwrap();
        let old_local = std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap();
        let foreign = b"workPath: foreign-project-work\n";
        let result = super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::GuardInstalled,
            move |path| {
                std::fs::rename(
                    path.join(super::EFFECTIVE_CONFIG_NAME),
                    path.join("owned-guard-retained.yaml"),
                )
                .unwrap();
                std::fs::write(path.join(super::EFFECTIVE_CONFIG_NAME), foreign).unwrap();
            },
            || write_projection(&mut store, &primary, Some(&after)),
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(path.join(super::EFFECTIVE_CONFIG_NAME)).unwrap(),
            foreign
        );
        assert!(serde_yaml::from_slice::<serde_yaml::Value>(
            &std::fs::read(path.join("owned-guard-retained.yaml")).unwrap()
        )
        .unwrap()
        .is_null());
        assert_eq!(
            std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap(),
            old_local
        );
    }

    #[test]
    fn runner_project_projection_foreign_local_in_the_gap_is_preserved_with_the_null_guard() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        write_projection(&mut store, &primary, Some(&before)).unwrap();
        let previous_local = std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap();
        let foreign = b"infobases: {foreign: {connection: 'File=/foreign/base'}}\n";
        let result = super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::LocalGap,
            move |path| {
                assert!(read_projection_primary(path).is_null());
                std::fs::write(path.join(super::LOCAL_CONFIG_NAME), foreign).unwrap();
            },
            || write_projection(&mut store, &primary, Some(&after)),
        );
        assert!(result.is_err());
        assert!(
            read_projection_primary(&path).is_null(),
            "a local refusal must not reinstate a valid primary"
        );
        assert_eq!(
            std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap(),
            foreign
        );
        assert!(std::fs::read_dir(&path).unwrap().flatten().any(|entry| {
            entry.file_name().to_string_lossy().starts_with(".config-recovery-")
                && std::fs::read(entry.path()).is_ok_and(|bytes| bytes == previous_local)
        }), "the old owned local layer must remain in recovery when a foreign file blocks restoration");
    }

    #[test]
    fn runner_project_projection_interrupted_local_removal_is_repaired_at_the_same_owner_path() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        write_projection(&mut store, &primary, Some(&before)).unwrap();
        let stable_config = store.config_path().to_path_buf();
        let result = super::with_runner_projection_failpoint(
            super::RunnerProjectionCheckpoint::LocalGap,
            || write_projection(&mut store, &primary, None),
        );
        assert!(result.is_err());
        assert!(read_projection_primary(&path).is_null());
        assert!(!path.join(super::LOCAL_CONFIG_NAME).exists());
        drop(store);
        let mut reopened =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        write_projection(&mut reopened, &primary, Some(&after)).unwrap();
        assert_eq!(reopened.config_path(), stable_config);
        assert_eq!(read_projection_primary(&path), primary);
        assert_eq!(
            serde_yaml::from_slice::<serde_yaml::Value>(
                &std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap()
            )
            .unwrap(),
            after
        );
    }

    #[test]
    fn runner_project_projection_leaves_an_unchanged_local_file_at_its_retained_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, local) = projection_values("unchanged");
        write_projection(&mut store, &primary, Some(&local)).unwrap();
        let local_path = path.join(super::LOCAL_CONFIG_NAME);
        let identity = super::file_identity(&std::fs::File::open(&local_path).unwrap()).unwrap();
        super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::GuardInstalled,
            |_| panic!("unchanged local must not install an update guard"),
            || write_projection(&mut store, &primary, Some(&local)),
        )
        .unwrap();
        assert_eq!(
            super::file_identity(&std::fs::File::open(&local_path).unwrap()).unwrap(),
            identity
        );
        assert_eq!(read_projection_primary(&path), primary);
    }

    #[test]
    fn runner_project_store_keeps_a_replaced_config_and_directory_untouched() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runner-project");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let value: serde_yaml::Value =
            serde_yaml::from_str("infobases: {origin: {password: secret}}\n").unwrap();
        store
            .write_config(super::LOCAL_CONFIG_NAME, &value)
            .unwrap();
        let local = path.join(super::LOCAL_CONFIG_NAME);
        std::fs::rename(&local, path.join("original-local.yaml")).unwrap();
        std::fs::write(&local, b"foreign replacement").unwrap();
        assert!(store.remove_config(super::LOCAL_CONFIG_NAME).is_err());
        assert_eq!(std::fs::read(&local).unwrap(), b"foreign replacement");
        let displaced = root.path().join("displaced-project");
        std::fs::rename(&path, &displaced).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("foreign.txt"), b"foreign directory").unwrap();
        assert!(store.remove_projection().is_err());
        assert_eq!(
            std::fs::read(path.join("foreign.txt")).unwrap(),
            b"foreign directory"
        );
        assert_eq!(
            std::fs::read(displaced.join("original-local.yaml")).unwrap(),
            serde_yaml::to_string(&value).unwrap().as_bytes()
        );
    }

    #[test]
    fn runner_project_store_does_not_overwrite_a_config_swapped_at_publication() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runner-project");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let before = serde_yaml::Value::String("original secret".into());
        store
            .write_config(super::LOCAL_CONFIG_NAME, &before)
            .unwrap();
        let local = path.join(super::LOCAL_CONFIG_NAME);
        let displaced = path.join("displaced-local.yaml");
        let hook_local = local.clone();
        let hook_displaced = displaced.clone();
        super::set_before_fixture_config_rename_hook(move || {
            std::fs::rename(&hook_local, &hook_displaced).unwrap();
            std::fs::write(&hook_local, b"foreign publication boundary").unwrap();
        });
        let result = store.write_config(
            super::LOCAL_CONFIG_NAME,
            &serde_yaml::Value::String("new secret".into()),
        );
        assert!(result.is_err(), "raced config publication must be refused");
        assert_eq!(
            std::fs::read(local).unwrap(),
            b"foreign publication boundary"
        );
        assert_eq!(
            std::fs::read(displaced).unwrap(),
            serde_yaml::to_string(&before).unwrap().as_bytes()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_runner_project_layers_are_owner_only_before_secret_bytes_are_written() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runner-project");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        for name in [super::EFFECTIVE_CONFIG_NAME, super::LOCAL_CONFIG_NAME] {
            let observed = std::rc::Rc::new(std::cell::Cell::new(false));
            let hook_observed = observed.clone();
            super::with_effective_config_create_hook(
                move |path| {
                    let file = std::fs::File::open(path).unwrap();
                    assert_eq!(file.metadata().unwrap().len(), 0);
                    super::verify_private_owner(&file).unwrap();
                    let directory = super::open_directory_nofollow(path.parent().unwrap()).unwrap();
                    super::verify_private_owner(&directory).unwrap();
                    hook_observed.set(true);
                },
                || {
                    store
                        .write_config(
                            name,
                            &serde_yaml::Value::String("private-test-secret".into()),
                        )
                        .unwrap()
                },
            );
            assert!(observed.get());
            let file = std::fs::File::open(path.join(name)).unwrap();
            super::verify_private_owner(&file).unwrap();
        }
    }

    #[test]
    fn runner_project_store_lock_survives_removal_and_does_not_lock_other_workspaces() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("workspace-a");
        let mut owner =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        write_projection(
            &mut owner,
            &serde_yaml::Value::Mapping(Default::default()),
            None,
        )
        .unwrap();
        owner.remove_projection().unwrap();
        assert!(!path.exists());

        let cancellation = CancellationToken::new();
        let contender_cancellation = cancellation.clone();
        let contender_path = path.clone();
        let (contended_tx, contended_rx) = std::sync::mpsc::channel();
        let contender = std::thread::spawn(move || {
            super::with_test_project_lock_contention(
                move || {
                    contended_tx.send(()).unwrap();
                },
                || {
                    super::RunnerProjectStore::open_and_lock(
                        &contender_path,
                        &contender_cancellation,
                    )
                    .err()
                },
            )
        });
        contended_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("same projection must retain the original lock after removal");
        cancellation.cancel();
        assert!(matches!(
            contender.join().unwrap().unwrap(),
            super::RunnerProjectLockFailure::Cancelled
        ));

        let independent_cancellation = CancellationToken::new();
        let child_cancellation = independent_cancellation.clone();
        let sibling = root.path().join("workspace-b");
        let (independent_tx, independent_rx) = std::sync::mpsc::channel();
        let independent = std::thread::spawn(move || {
            let acquired = super::RunnerProjectStore::open_and_lock(&sibling, &child_cancellation);
            independent_tx.send(acquired.is_ok()).unwrap();
        });
        let acquired = independent_rx.recv_timeout(std::time::Duration::from_secs(5));
        independent_cancellation.cancel();
        independent.join().unwrap();
        assert!(
            acquired.unwrap(),
            "another workspace must have independent ownership"
        );
        drop(owner);
        super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    }

    #[test]
    fn installed_tree_interior_mutation_after_first_probe_is_quarantined_and_rolled_back() {
        let (root, context, _) = workspace("installed-tree-interior-mutation");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::write(hook_target.join("unvalidated.txt"), b"unvalidated").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("before.txt")).unwrap(),
            b"before",
            "the original target must be restored after mutation of our installed directory"
        );
        assert!(
            !target.join("unvalidated.txt").exists(),
            "our installed but mutated directory must be quarantined before lock release"
        );
        assert!(
            result.errors.join("\n").contains("published dump differs")
                || result.errors.join("\n").contains("quarantine"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_preparation_failure_reports_retained_projection_and_private_data() {
        let (root, context, config) = workspace("canonical-prepare-retention");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let public_before = super::capture_tree_target_nofollow(&target).unwrap();
        let runner = DumpRunner::valid();
        let cancellation = CancellationToken::new();
        let store =
            super::RunnerProjectStore::open_and_lock(&root.join("projection"), &cancellation)
                .unwrap();
        let retained = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_retained = std::rc::Rc::clone(&retained);
        let hook_workspace = context.cwd.clone();
        let result = super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::LocalConfirmed,
            move |store_path| {
                let projection = store_path.join(super::EFFECTIVE_CONFIG_NAME);
                std::fs::write(&projection, b"foreign projection sentinel").unwrap();
                let projection_identity =
                    super::file_identity(&std::fs::File::open(&projection).unwrap()).unwrap();
                let mut private_roots = std::fs::read_dir(&hook_workspace)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .collect::<Vec<_>>();
                assert_eq!(private_roots.len(), 1);
                let private = private_roots.remove(0);
                let foreign = private.join("execution").join("foreign-source");
                std::fs::create_dir(&foreign).unwrap();
                std::fs::write(foreign.join("sentinel.txt"), b"foreign source sentinel").unwrap();
                let snapshot = super::capture_tree_target_nofollow(&private).unwrap();
                *hook_retained.borrow_mut() =
                    Some((private, snapshot, projection, projection_identity));
            },
            || {
                super::prepare_publication(
                    &context,
                    super::FullDumpSelection {
                        config_path: &config,
                        source_set: Some("main"),
                        extension: None,
                        expected_target: None,
                    },
                    store,
                    &runner,
                    &cancellation,
                    |mut primary, local, _| {
                        if let Some(local) = local {
                            super::merge_yaml_values(&mut primary, local);
                        }
                        Ok(primary)
                    },
                    |effective, _| Ok((effective, None)),
                    crate::infrastructure::runtime_composition::full_dump_semantics(),
                )
            },
        );
        let failure = match result {
            Ok(publication) => {
                drop(publication);
                panic!("a foreign projection must refuse canonical preparation");
            }
            Err(failure) => failure,
        };
        assert!(matches!(failure.phase, super::FullDumpPhase::Preparation));
        assert!(failure
            .test_detail()
            .contains("local config overlay appeared during full dump"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            public_before
        );
        assert!(
            !failure.publication_uncertain,
            "private preparation retention must not claim an unknown public publication"
        );
        let retained = retained.borrow();
        let (private, snapshot, projection, projection_identity) =
            retained.as_ref().expect("projection hook must run");
        assert_eq!(
            super::capture_tree_target_nofollow(private).unwrap(),
            *snapshot,
            "preparation cleanup must preserve all private identities and bytes"
        );
        assert_eq!(
            std::fs::read(private.join("execution/foreign-source/sentinel.txt")).unwrap(),
            b"foreign source sentinel"
        );
        assert_eq!(
            super::file_identity(&std::fs::File::open(projection).unwrap()).unwrap(),
            *projection_identity,
            "the rejected foreign projection must retain its identity"
        );
        assert_eq!(
            std::fs::read(projection).unwrap(),
            b"foreign projection sentinel"
        );
        let recovery = failure
            .recovery
            .as_ref()
            .expect("retained preparation location must remain identity-proven")
            .as_ref()
            .expect("retained canonical preparation data needs a recovery location");
        assert_eq!(
            std::fs::canonicalize(recovery).unwrap(),
            std::fs::canonicalize(private).unwrap()
        );
        let expected_identity = match snapshot {
            super::TreeSnapshot::Directory { identity, .. } => *identity,
            super::TreeSnapshot::Absent => panic!("the captured private root must be present"),
        };
        assert_eq!(
            super::file_identity(
                &super::open_directory_nofollow(&std::fs::canonicalize(recovery).unwrap()).unwrap(),
            )
            .unwrap(),
            expected_identity,
            "recovery must identify the actual retained private owner"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn verify_canonical_ready_owned_output_cleanup(platform_changed: bool) {
        let (root, context, config) = workspace("canonical-ready-owned-output");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let target_before = TreeSnapshot::capture_target(&target).unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        publication.begin_execution();
        let stage = publication.staged_target().to_path_buf();
        let private_root = publication.test_observation().private_root;
        // The test owns this synchronous writer. Do not use fill_canonical_stage:
        // that helper also changes the private cleanup ledger and masks the gap.
        std::fs::write(
            stage.join("Configuration.xml"),
            if platform_changed {
                valid_configuration_owner()
            } else {
                "<invalid-owned-xml>".to_string()
            },
        )
        .unwrap();
        std::fs::write(stage.join("payload.bsl"), b"owned completed writer bytes").unwrap();
        if platform_changed {
            std::fs::write(&platform.executable, b"changed after Selected").unwrap();
        }
        let failure = publication.publish(&cancellation).unwrap_err();
        assert!(
            matches!(
                failure.phase,
                super::FullDumpPhase::Integrity if platform_changed
            ) || matches!(failure.phase, super::FullDumpPhase::Validation) && !platform_changed
        );
        assert!(publication.test_observation().writer_completion_pending);
        assert!(!cancellation.protected_process_started());
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            target_before
        );
        // This test's writer returned synchronously above. This explicit proof is
        // confined to the guard test; ProcessStreamOutput does not supply it.
        publication.confirm_execution_finished();
        let finish = publication.finish();
        assert!(finish.is_ok(), "known-finished owned output must clean");
        let finish = finish.unwrap();
        assert!(finish.warnings.is_empty());
        assert!(finish.retained.unwrap().is_none());
        assert!(!private_root.exists());
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            target_before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_ready_invalid_owned_xml_cleans_after_test_owned_writer_finishes() {
        verify_canonical_ready_owned_output_cleanup(false);
    }

    #[test]
    fn canonical_ready_platform_recheck_refusal_cleans_after_test_owned_writer_finishes() {
        verify_canonical_ready_owned_output_cleanup(true);
    }

    fn write_canonical_ready_output(publication: &super::VerifiedFullDumpPublication) {
        std::fs::write(
            publication.staged_target().join("Configuration.xml"),
            valid_configuration_owner(),
        )
        .unwrap();
        std::fs::write(
            publication.staged_target().join("payload.bsl"),
            b"owned completed writer bytes",
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_ready_root_replacement_after_capture_remains_unowned() {
        let (root, context, config) = workspace("canonical-ready-root-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let target_before = TreeSnapshot::capture_target(&target).unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        publication.begin_execution();
        write_canonical_ready_output(&publication);
        let stage = publication.staged_target().to_path_buf();
        let displaced = root.join("displaced-owned-ready-stage");
        let owned_before = TreeSnapshot::capture_target(&stage).unwrap();
        let hook_stage = stage.clone();
        let hook_displaced = displaced.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::AfterStagedCapture,
            move || {
                std::fs::rename(&hook_stage, &hook_displaced).unwrap();
                std::fs::create_dir(&hook_stage).unwrap();
                std::fs::write(hook_stage.join("foreign.txt"), b"foreign after Ready").unwrap();
            },
            || publication.publish(&cancellation),
        );
        assert!(result.is_err(), "a replaced Ready root cannot publish");
        assert!(matches!(
            result.unwrap_err().phase,
            super::FullDumpPhase::Integrity
        ));
        let foreign_before = TreeSnapshot::capture_target(&stage).unwrap();
        assert!(publication.test_observation().writer_completion_pending);
        assert!(!cancellation.protected_process_started());
        publication.confirm_execution_finished();
        let finish = publication.finish();
        assert!(finish.is_err());
        let failure = match finish {
            Err(failure) => failure,
            Ok(_) => unreachable!(),
        };
        assert!(matches!(failure.phase, super::FullDumpPhase::Integrity));
        assert_eq!(
            TreeSnapshot::capture_target(&stage).unwrap(),
            foreign_before
        );
        assert_eq!(
            TreeSnapshot::capture_target(&displaced).unwrap(),
            owned_before
        );
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            target_before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_ready_same_inode_mutation_after_capture_is_not_recaptured() {
        let (root, context, config) = workspace("canonical-ready-content-mutation");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let target_before = TreeSnapshot::capture_target(&target).unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        publication.begin_execution();
        write_canonical_ready_output(&publication);
        let stage = publication.staged_target().to_path_buf();
        let payload = stage.join("payload.bsl");
        let payload_before = super::file_identity(&std::fs::File::open(&payload).unwrap()).unwrap();
        let hook_payload = payload.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::AfterStagedCapture,
            move || std::fs::write(&hook_payload, b"late same-inode replacement bytes").unwrap(),
            || publication.publish(&cancellation),
        );
        assert!(result.is_err(), "Ready bytes cannot be silently refreshed");
        assert!(matches!(
            result.unwrap_err().phase,
            super::FullDumpPhase::Publication
        ));
        assert_eq!(
            super::file_identity(&std::fs::File::open(&payload).unwrap()).unwrap(),
            payload_before
        );
        assert!(publication.test_observation().writer_completion_pending);
        assert!(!cancellation.protected_process_started());
        publication.confirm_execution_finished();
        let finished = publication.finish().unwrap();
        assert!(matches!(&finished.retained, Ok(None)));
        assert!(finished.warnings.is_empty());
        assert_eq!(
            std::fs::symlink_metadata(&stage).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            target_before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_published_repeat_never_adopts_a_new_stage_name() {
        let (root, context, config) = workspace("canonical-published-foreign-stage");
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        publication.begin_execution();
        write_canonical_ready_output(&publication);
        let stage = publication.staged_target().to_path_buf();
        let published = publication.publish(&cancellation).unwrap();
        assert_eq!(published.files, 2);
        assert!(publication.test_observation().writer_completion_pending);
        let target_before = TreeSnapshot::capture_target(publication.target()).unwrap();
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("foreign.txt"), b"foreign after publication").unwrap();
        let foreign_before = TreeSnapshot::capture_target(&stage).unwrap();
        assert_eq!(publication.publish(&cancellation).unwrap().files, 2);
        assert_eq!(
            TreeSnapshot::capture_target(publication.target()).unwrap(),
            target_before
        );
        publication.confirm_execution_finished();
        let finish = publication.finish().unwrap();
        assert!(!finish.warnings.is_empty());
        assert_eq!(
            TreeSnapshot::capture_target(&stage).unwrap(),
            foreign_before
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
