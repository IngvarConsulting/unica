use super::persistent_provider_cwd_command_tests::{argument, CwdFixture};
use super::*;
use crate::infrastructure::platform::testing;
use crate::test_support::canonical_path;
use std::ffi::{OsStr, OsString};
use std::process::Command;

#[derive(Debug)]
struct Observation {
    pid: u32,
    cwd: PathBuf,
    generation: OsString,
    args: Vec<OsString>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

#[cfg(unix)]
fn decode_native(value: &str) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    assert_eq!(value.len() % 2, 0);
    OsString::from_vec(
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect(),
    )
}
#[cfg(windows)]
fn decode_native(value: &str) -> OsString {
    use std::os::windows::ffi::OsStringExt;
    assert_eq!(value.len() % 4, 0);
    let units = (0..value.len())
        .step_by(4)
        .map(|index| u16::from_str_radix(&value[index..index + 4], 16).unwrap())
        .collect::<Vec<_>>();
    OsString::from_wide(&units)
}

fn observation(text: &str) -> Observation {
    let fields = text.trim().split(';').collect::<Vec<_>>();
    assert_eq!(fields.len(), 6, "native probe observation: {text:?}");
    Observation {
        pid: fields[0].parse().unwrap(),
        cwd: PathBuf::from(decode_native(fields[1])),
        generation: decode_native(fields[2]),
        args: if fields[5].is_empty() {
            vec![]
        } else {
            fields[5].split(',').map(decode_native).collect()
        },
        #[cfg(unix)]
        dev: fields[3].parse().expect("Unix cwd dev is mandatory"),
        #[cfg(unix)]
        ino: fields[4].parse().expect("Unix cwd ino is mandatory"),
    }
}

fn assert_actual_cwd(actual: &Observation, expected: &Path) {
    assert_eq!(canonical_path(&actual.cwd), canonical_path(expected));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(expected).unwrap();
        assert_eq!(
            (actual.dev, actual.ino),
            (metadata.dev(), metadata.ino()),
            "Unix cwd evidence is object identity, not rename success"
        );
    }
}

fn assert_outside(actual: &Observation, workspace: &Path, source: &Path) {
    let cwd = canonical_path(&actual.cwd);
    for inside in [workspace, source] {
        assert!(
            !crate::infrastructure::platform::filesystem::path_starts_with_host_root(
                &cwd,
                &canonical_path(inside)
            ),
            "actual persistent cwd {cwd:?} is inside {inside:?}"
        );
    }
    assert_actual_cwd(actual, &cwd);
}

fn compile_probe(fixture: &CwdFixture) -> PathBuf {
    let bin_dir = fixture.root.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let binary = testing::fixture_executable_path(&bin_dir, "persistent-provider-cwd-probe");
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/platform/fixtures/persistent_provider_cwd_probe.rs");
    assert!(!source.starts_with(&fixture.context.workspace_root));
    assert!(!binary.starts_with(&fixture.context.workspace_root));
    let mut command = Command::new(env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc")));
    command
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .current_dir(&fixture.external);
    let mut compiler = ManagedChild::spawn_process(
        command,
        Some(Duration::from_secs(30)),
        CancellationToken::new(),
    )
    .unwrap();
    let output = compiler.wait_for_output().unwrap();
    assert!(
        output.status_success && !output.timed_out && !output.cancelled,
        "probe compiler: {output:?}"
    );
    assert!(!output.stdout_truncated && !output.stderr_truncated);
    binary
}

fn probe(session: &mut PersistentMcpSession) -> Observation {
    let output = session
        .call(
            "cwd_probe",
            json!({}),
            Duration::from_secs(3),
            &CancellationToken::new(),
        )
        .unwrap();
    observation(&output.result_text)
}

fn stop_session(session: &mut PersistentMcpSession) {
    session
        .retire_for_replacement(
            &ElapsedBudgetDeadline::new(Duration::from_secs(2)),
            &CancellationToken::new(),
        )
        .unwrap();
    assert!(
        session.child.poll_owned_leader_exit().unwrap(),
        "owned probe leader must be reaped before fixture cleanup"
    );
}

fn live_provider_rename(rlm: bool) {
    let fixture = CwdFixture::new(true);
    let binary = compile_probe(&fixture);
    let command = if rlm {
        rlm_transport_command_for_build(binary, &fixture.context, &fixture.source, &fixture.build)
            .unwrap()
    } else {
        bsl_analyzer_command(&binary, &fixture.context, &fixture.source).unwrap()
    };
    let expected_args = command
        .get_args()
        .map(OsStr::to_os_string)
        .collect::<Vec<_>>();
    let configured = command.get_current_dir().map(Path::to_path_buf);
    let expected_cache = (!rlm).then(|| argument(&command, "--cache-dir"));
    let mut session =
        PersistentMcpSession::start_with_command(command, &CancellationToken::new()).unwrap();
    let before = probe(&mut session);
    assert_eq!(before.pid, session.child.id());
    assert_eq!(before.args, expected_args);
    assert_outside(&before, &fixture.context.workspace_root, &fixture.source);
    assert_actual_cwd(
        &before,
        configured
            .as_deref()
            .expect("producer must choose explicit cwd"),
    );
    if rlm {
        assert_eq!(before.generation.as_os_str(), fixture.build.as_os_str());
    } else {
        let index = before
            .args
            .iter()
            .position(|arg| arg.as_os_str() == OsStr::new("--source-dir"))
            .unwrap();
        assert_eq!(
            PathBuf::from(before.args[index + 1].as_os_str()),
            canonical_path(&fixture.source)
        );
        let index = before
            .args
            .iter()
            .position(|arg| arg.as_os_str() == OsStr::new("--cache-dir"))
            .unwrap();
        assert_eq!(Some(&before.args[index + 1]), expected_cache.as_ref());
    }
    assert!(session.is_reusable());
    let renamed = fixture.root.join("workspace-renamed");
    fs::rename(&fixture.context.workspace_root, &renamed).unwrap();
    assert!(
        session.is_reusable(),
        "rename cannot be achieved by retiring the warm provider"
    );
    let after = probe(&mut session);
    assert_eq!(
        after.pid, before.pid,
        "same live process before and after rename"
    );
    assert_actual_cwd(&after, &before.cwd);
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.args, before.args);
    assert!(session.is_reusable());
    stop_session(&mut session);
}

#[test]
fn native_rlm_cwd_is_external_while_same_warm_process_survives_workspace_rename() {
    live_provider_rename(true);
}

#[test]
fn native_bsl_cwd_is_external_while_same_warm_process_survives_workspace_rename() {
    live_provider_rename(false);
}

#[cfg(windows)]
#[test]
fn windows_factories_start_live_providers_at_an_external_anchor_when_other_candidates_are_excluded()
{
    let mut fixture = CwdFixture::new(true);
    let binary = compile_probe(&fixture);
    let temp = canonical_path(&env::temp_dir());
    let executable = canonical_path(env::current_exe().unwrap().parent().unwrap());
    fixture.context.workspace_root = temp.clone();
    let identity = WorkspaceServiceIdentity::new(&fixture.context, &executable).unwrap();
    let commands = [
        rlm_transport_command_for_build(
            binary.clone(),
            &fixture.context,
            &executable,
            &fixture.build,
        )
        .unwrap(),
        bsl_analyzer_command(&binary, &fixture.context, &executable).unwrap(),
        workspace_service_command(
            &binary,
            &identity,
            WorkspaceServiceConfig { idle_secs: 321 },
            "anchor-test-token",
            &CoreIdentity::production(),
        )
        .unwrap(),
    ];
    let anchor = temp.ancestors().last().unwrap();
    for command in commands {
        let expected_args = command
            .get_args()
            .map(OsStr::to_os_string)
            .collect::<Vec<_>>();
        assert_eq!(canonical_path(command.get_current_dir().unwrap()), anchor);
        let mut session =
            PersistentMcpSession::start_with_command(command, &CancellationToken::new()).unwrap();
        let before = probe(&mut session);
        assert_eq!(before.pid, session.child.id());
        assert_eq!(before.args, expected_args);
        assert_outside(&before, &temp, &executable);
        assert_actual_cwd(&before, anchor);
        assert_eq!(probe(&mut session).pid, before.pid);
        stop_session(&mut session);
    }
}

#[cfg(windows)]
#[test]
fn windows_live_cwd_inside_outside_pair_isolates_workspace_rename() {
    let fixture = CwdFixture::new(false);
    let binary = compile_probe(&fixture);
    let make = || {
        rlm_transport_command_for_build(
            binary.clone(),
            &fixture.context,
            &fixture.source,
            &fixture.build,
        )
        .unwrap()
    };
    let mut inside_command = make();
    inside_command.current_dir(&fixture.context.workspace_root);
    let mut outside_command = make();
    outside_command.current_dir(&fixture.external);
    assert_eq!(
        inside_command.get_args().collect::<Vec<_>>(),
        outside_command.get_args().collect::<Vec<_>>()
    );
    assert_eq!(
        inside_command.get_envs().collect::<Vec<_>>(),
        outside_command.get_envs().collect::<Vec<_>>()
    );
    let mut inside =
        PersistentMcpSession::start_with_command(inside_command, &CancellationToken::new())
            .unwrap();
    let before = probe(&mut inside);
    assert_actual_cwd(&before, &fixture.context.workspace_root);
    assert!(inside.is_reusable());
    let renamed = fixture.root.join("workspace-renamed");
    let error = fs::rename(&fixture.context.workspace_root, &renamed)
        .expect_err("live inside cwd must block Windows rename");
    assert!(
        matches!(error.raw_os_error(), Some(5 | 32 | 33)),
        "unexpected rename refusal: {error}"
    );
    assert_eq!(probe(&mut inside).pid, before.pid);
    assert!(inside.is_reusable());
    stop_session(&mut inside);
    let mut outside =
        PersistentMcpSession::start_with_command(outside_command, &CancellationToken::new())
            .unwrap();
    let before = probe(&mut outside);
    assert_actual_cwd(&before, &fixture.external);
    assert!(outside.is_reusable());
    fs::rename(&fixture.context.workspace_root, &renamed).unwrap();
    let after = probe(&mut outside);
    assert_eq!(after.pid, before.pid);
    assert_actual_cwd(&after, &fixture.external);
    assert!(outside.is_reusable());
    stop_session(&mut outside);
}

#[test]
fn native_helper_does_not_inherit_project_cwd_from_isolated_caller() {
    let fixture = CwdFixture::new(true);
    let binary = compile_probe(&fixture);
    let mut command = Command::new(env::current_exe().unwrap());
    command.args(["--exact", "infrastructure::workspace_services::persistent_provider_cwd_native_tests::helper_inherited_cwd_subprocess_fixture",
                  "--ignored", "--nocapture"])
        .current_dir(&fixture.context.cwd)
        .env("UNICA_CWD_HELPER_FIXTURE_ROOT", &fixture.root)
        .env("UNICA_CWD_HELPER_FIXTURE_EXE", binary);
    let mut child = ManagedChild::spawn_process(
        command,
        Some(Duration::from_secs(30)),
        CancellationToken::new(),
    )
    .unwrap();
    let output = child.wait_for_output().unwrap();
    assert!(
        output.status_success && !output.timed_out && !output.cancelled,
        "isolated helper proof: {output:?}"
    );
    assert!(!output.stdout_truncated && !output.stderr_truncated);
}

fn wait_observation(path: &Path, child: &mut ManagedStartupChild) -> Observation {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            child.is_running().unwrap(),
            "owned helper exited before observation: {path:?}"
        );
        if let Ok(text) = fs::read_to_string(path) {
            // A writer may still be finishing the tiny initial file. Do not parse a partial record.
            if text.ends_with('\n') && text.split(';').count() == 6 {
                return observation(&text);
            }
        }
        assert!(
            Instant::now() < deadline,
            "helper observation deadline: {path:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "owned subprocess fixture, invoked by native_helper test with exact private inputs"]
fn helper_inherited_cwd_subprocess_fixture() {
    let root = canonical_path(Path::new(
        &env::var_os("UNICA_CWD_HELPER_FIXTURE_ROOT").expect("private fixture root"),
    ));
    let binary =
        PathBuf::from(env::var_os("UNICA_CWD_HELPER_FIXTURE_EXE").expect("external probe binary"));
    let workspace = root.join("workspace");
    let source = workspace.join("src");
    let control = root.join("control");
    assert!(
        canonical_path(&env::current_dir().unwrap()).starts_with(&workspace),
        "isolated caller must begin inside project"
    );
    assert!(!binary.starts_with(&workspace));
    let context = WorkspaceContext {
        cwd: env::current_dir().unwrap(),
        workspace_root: workspace.clone(),
        cache_root: root.join("cache"),
        workspace_epoch: 1,
    };
    let identity = WorkspaceServiceIdentity::new(&context, &source).unwrap();
    let mut command = workspace_service_command(
        &binary,
        &identity,
        WorkspaceServiceConfig { idle_secs: 321 },
        "cwd-test-token",
        &CoreIdentity::production(),
    )
    .unwrap();
    let ready = control.join("helper-ready");
    let ping = control.join("helper-ping");
    let pong = control.join("helper-pong");
    let stop = control.join("helper-stop");
    command
        .env("UNICA_CWD_PROBE_HELPER", "1")
        .env("UNICA_CWD_PROBE_READY", &ready)
        .env("UNICA_CWD_PROBE_PING", &ping)
        .env("UNICA_CWD_PROBE_PONG", &pong)
        .env("UNICA_CWD_PROBE_STOP", &stop)
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            fs::File::create(control.join("helper.stdout")).unwrap(),
        ))
        .stderr(Stdio::from(
            fs::File::create(control.join("helper.stderr")).unwrap(),
        ));
    let expected_args = command
        .get_args()
        .map(OsStr::to_os_string)
        .collect::<Vec<_>>();
    let mut child = ManagedStartupChild::spawn_configured(command).unwrap();
    // This exact libtest subprocess runs only this fixture. The shared parent
    // test process never changes cwd/env. Release the caller's own cwd hold so
    // a Windows rename can identify the helper as the remaining cause.
    env::set_current_dir(&control).unwrap();
    assert_eq!(
        canonical_path(&env::current_dir().unwrap()),
        canonical_path(&control)
    );
    assert!(!canonical_path(&env::current_dir().unwrap()).starts_with(&workspace));
    let before = wait_observation(&ready, &mut child);
    assert_eq!(before.pid, child.id());
    assert_eq!(before.args, expected_args);
    assert_outside(&before, &workspace, &source);
    assert!(child.is_running().unwrap());
    fs::rename(&workspace, root.join("workspace-renamed")).unwrap();
    assert!(child.is_running().unwrap());
    fs::write(&ping, b"challenge-after-rename").unwrap();
    let after = wait_observation(&pong, &mut child);
    assert_eq!(after.pid, before.pid);
    assert_eq!(after.args, before.args);
    assert_actual_cwd(&after, &before.cwd);
    assert!(child.is_running().unwrap());
    fs::write(stop, b"stop").unwrap();
    child.terminate_bounded(Duration::from_secs(2)).unwrap();
}
