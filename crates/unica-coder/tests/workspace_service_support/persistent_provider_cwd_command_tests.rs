use super::*;
use std::ffi::{OsStr, OsString};
use unica_workspace::test_support::canonical_path;

pub(super) struct CwdFixture {
    _owner: tempfile::TempDir,
    pub(super) root: PathBuf,
    pub(super) context: WorkspaceContext,
    pub(super) source: PathBuf,
    pub(super) external: PathBuf,
    pub(super) build: PathBuf,
}

impl CwdFixture {
    pub(super) fn new(nested: bool) -> Self {
        let owner = tempfile::tempdir().unwrap();
        let root = canonical_path(owner.path());
        let workspace = root.join("workspace");
        let source = workspace.join("src");
        let external = root.join("control");
        let build = root.join("generation");
        let cwd = if nested {
            workspace.join("nested/deeper")
        } else {
            workspace.clone()
        };
        for directory in [&source, &external, &build, &cwd] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::write(
            source.join("Module.bsl"),
            "Procedure CwdProbe() Export\nEndProcedure\n",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd,
            workspace_root: workspace,
            cache_root: root.join("cache"),
            workspace_epoch: 1,
        };
        Self {
            _owner: owner,
            root,
            context,
            source,
            external,
            build,
        }
    }
}

pub(super) fn argument(command: &Command, flag: &str) -> OsString {
    let values = command
        .get_args()
        .map(OsStr::to_os_string)
        .collect::<Vec<_>>();
    let positions = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (value.as_os_str() == OsStr::new(flag)).then_some(index))
        .collect::<Vec<_>>();
    assert_eq!(
        positions.len(),
        1,
        "one explicit {flag} required: {values:?}"
    );
    values[positions[0] + 1].clone()
}

fn assert_external(command: &Command, fixture: &CwdFixture) {
    let configured = command
        .get_current_dir()
        .expect("persistent command must set cwd before spawn");
    assert!(
        configured.is_absolute() && configured.is_dir(),
        "cwd must be usable before spawn: {configured:?}"
    );
    let actual = canonical_path(configured);
    for forbidden in [&fixture.context.workspace_root, &fixture.source] {
        assert!(
            !unica_workspace::paths::path_starts_with_host_root(
                &actual,
                &canonical_path(forbidden)
            ),
            "persistent cwd {actual:?} is inside {forbidden:?}"
        );
    }
}

#[test]
fn rlm_command_sets_external_cwd_and_preserves_explicit_source_and_generation() {
    for (nested, cache_inside) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut fixture = CwdFixture::new(nested);
        if cache_inside {
            fixture.context.cache_root = fixture.context.workspace_root.join(".cache");
        }
        let command = rlm_transport_command_for_build(
            PathBuf::from("not-executed-rlm"),
            &fixture.context,
            &fixture.source,
            &fixture.build,
        )
        .unwrap();
        assert_external(&command, &fixture);
        let environment = command.get_envs().collect::<HashMap<_, _>>();
        assert_eq!(
            environment[OsStr::new("RLM_INDEX_DIR")],
            Some(fixture.build.as_os_str())
        );
        assert_eq!(environment[OsStr::new("PYTHONUTF8")], Some(OsStr::new("1")));
        assert_eq!(
            environment[OsStr::new("PYTHONIOENCODING")],
            Some(OsStr::new("utf-8:surrogateescape"))
        );
        let start =
            rlm_start_arguments(&fixture.source, "CwdProbe", Duration::from_secs(2)).unwrap();
        assert_eq!(start["path"], fixture.source.to_str().unwrap());
    }
}

#[test]
fn bsl_command_sets_external_cwd_and_preserves_source_specific_cache() {
    for (nested, cache_inside) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut fixture = CwdFixture::new(nested);
        if cache_inside {
            fixture.context.cache_root = fixture.context.workspace_root.join(".cache");
        }
        let second = fixture.context.workspace_root.join("extension");
        fs::create_dir_all(&second).unwrap();
        let command = bsl_analyzer_command(
            Path::new("not-executed-bsl"),
            &fixture.context,
            &fixture.source,
        )
        .unwrap();
        assert_external(&command, &fixture);
        assert_eq!(
            PathBuf::from(argument(&command, "--source-dir")),
            canonical_path(&fixture.source)
        );
        let cache = PathBuf::from(argument(&command, "--cache-dir"));
        assert!(cache.is_absolute());
        assert!(!cache.starts_with(&fixture.source));
        let other =
            bsl_analyzer_command(Path::new("not-executed-bsl"), &fixture.context, &second).unwrap();
        assert_ne!(
            argument(&command, "--cache-dir"),
            argument(&other, "--cache-dir")
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == OsStr::new("BSL_MCP_BROKER"))
                .unwrap()
                .1,
            Some(OsStr::new("0"))
        );
    }
}

#[test]
fn helper_command_sets_external_cwd_and_preserves_explicit_registration() {
    for (nested, cache_inside) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut fixture = CwdFixture::new(nested);
        if cache_inside {
            fixture.context.cache_root = fixture.context.workspace_root.join(".cache");
        }
        let identity = WorkspaceServiceIdentity::new(&fixture.context, &fixture.source).unwrap();
        let build = CoreIdentity::production();
        let command = workspace_service_command(
            Path::new("not-executed-helper"),
            &identity,
            WorkspaceServiceConfig { idle_secs: 321 },
            "cwd-test-token",
            &build,
        )
        .unwrap();
        assert_external(&command, &fixture);
        assert_eq!(
            argument(&command, "--workspace-root").as_os_str(),
            OsStr::new(&identity.workspace_root)
        );
        assert_eq!(
            argument(&command, "--source-root").as_os_str(),
            OsStr::new(&identity.source_root)
        );
        assert_eq!(
            PathBuf::from(argument(&command, "--service-dir")),
            identity.service_dir
        );
        assert_eq!(
            argument(&command, "--core-identity").as_os_str(),
            OsStr::new(build.as_str())
        );
        assert_eq!(
            argument(&command, "--token").as_os_str(),
            OsStr::new("cwd-test-token")
        );
        assert_eq!(
            argument(&command, "--idle-secs").as_os_str(),
            OsStr::new("321")
        );
    }
}
