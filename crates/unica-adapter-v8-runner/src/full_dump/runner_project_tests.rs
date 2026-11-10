//! The runner project store of a working copy and its projection: the private
//! effective configuration v8-runner reads and the lock serializing its runs.
use super::write_projection;
use std::path::Path;
use unica_work_control::cancellation::CancellationToken;
#[cfg(windows)]
use unica_workspace::full_dump::test_support::with_effective_config_create_hook;
use unica_workspace::full_dump::test_support::{
    fixture_file_identity, fixture_verify_private_owner, set_before_fixture_config_rename_hook,
    with_runner_projection_failpoint, with_runner_projection_hook,
    with_test_project_lock_contention, RunnerProjectionCheckpoint,
};
use unica_workspace::full_dump::{RunnerProjectLockFailure, RunnerProjectStore};
use unica_workspace::read::{
    open_read_directory_nofollow as open_directory_nofollow, FileIdentity, ReadFile,
};

const EFFECTIVE_CONFIG_NAME: &str = "v8project.yaml";
const LOCAL_CONFIG_NAME: &str = "v8project.local.yaml";

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

fn projection_values(password: &str) -> (serde_yaml::Value, serde_yaml::Value) {
    let primary =
        serde_yaml::from_str("workPath: .work\nformat: DESIGNER\nsource-set: []\n").unwrap();
    let local = serde_yaml::from_str(&format!(
        "infobases:\n  origin:\n    connection: 'File=/retained/owned/base'\n    password: '{password}'\n")).unwrap();
    (primary, local)
}

fn read_projection_primary(path: &Path) -> serde_yaml::Value {
    serde_yaml::from_slice(&std::fs::read(path.join(EFFECTIVE_CONFIG_NAME)).unwrap()).unwrap()
}

#[test]
fn runner_project_projection_keeps_a_null_primary_through_the_local_gap() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("projection");
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let (primary, before) = projection_values("before");
    let (_, after) = projection_values("after");
    write_projection(&mut store, &primary, Some(&before)).unwrap();
    let observed = std::rc::Rc::new(std::cell::Cell::new(false));
    let hook_observed = observed.clone();
    with_runner_projection_hook(
        RunnerProjectionCheckpoint::LocalGap,
        move |path| {
            assert!(
                !path.join(LOCAL_CONFIG_NAME).exists(),
                "the hook must observe the real displacement gap"
            );
            assert!(
                read_projection_primary(path).is_null(),
                "owner guard must be unreadable while the local layer is absent"
            );
            let guard = std::fs::File::open(path.join(EFFECTIVE_CONFIG_NAME)).unwrap();
            verify_private_owner(&guard).unwrap();
            hook_observed.set(true);
        },
        || write_projection(&mut store, &primary, Some(&after)),
    )
    .unwrap();
    assert!(observed.get());
    assert_eq!(read_projection_primary(&path), primary);
    let projected: serde_yaml::Value =
        serde_yaml::from_slice(&std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap()).unwrap();
    assert_eq!(projected, after);
    store.verify().unwrap();
}

#[test]
fn runner_project_projection_removal_keeps_the_guard_until_local_absence_is_confirmed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("projection");
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let (primary, local) = projection_values("before");
    write_projection(&mut store, &primary, Some(&local)).unwrap();
    let observed = std::rc::Rc::new(std::cell::Cell::new(false));
    let hook_observed = observed.clone();
    with_runner_projection_hook(
        RunnerProjectionCheckpoint::LocalGap,
        move |path| {
            assert!(!path.join(LOCAL_CONFIG_NAME).exists());
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
    assert!(!path.join(LOCAL_CONFIG_NAME).exists());
    store.verify().unwrap();
}

#[test]
fn runner_project_projection_failure_after_guard_stays_unreadable_until_next_owned_repair() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("projection");
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let (primary, before) = projection_values("before");
    let (_, after) = projection_values("after");
    write_projection(&mut store, &primary, Some(&before)).unwrap();
    let stable_config = store.config_path().to_path_buf();
    let previous_local = std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap();
    let result =
        with_runner_projection_failpoint(RunnerProjectionCheckpoint::GuardInstalled, || {
            write_projection(&mut store, &primary, Some(&after))
        });
    assert!(result.is_err());
    assert!(read_projection_primary(&path).is_null());
    assert_eq!(
        std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap(),
        previous_local,
        "failure must not delete the old local layer"
    );
    store.verify().unwrap();
    write_projection(&mut store, &primary, Some(&after)).unwrap();
    assert_eq!(store.config_path(), stable_config);
    assert_eq!(read_projection_primary(&path), primary);
    let repaired: serde_yaml::Value =
        serde_yaml::from_slice(&std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap()).unwrap();
    assert_eq!(repaired, after);
    assert_eq!(
        repaired["infobases"]["origin"]["connection"], before["infobases"]["origin"]["connection"],
        "repair keeps the same source/infobase ownership recipe"
    );
}

#[test]
fn runner_project_projection_error_in_local_gap_retains_guard_and_restores_old_owned_bytes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("projection");
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let (primary, before) = projection_values("before");
    let (_, after) = projection_values("after");
    write_projection(&mut store, &primary, Some(&before)).unwrap();
    let previous_local = std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap();
    let result = with_runner_projection_failpoint(RunnerProjectionCheckpoint::LocalGap, || {
        write_projection(&mut store, &primary, Some(&after))
    });
    assert!(result.is_err());
    assert!(read_projection_primary(&path).is_null());
    assert_eq!(
        std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap(),
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
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let (primary, before) = projection_values("before");
    let (_, after) = projection_values("after");
    write_projection(&mut store, &primary, Some(&before)).unwrap();
    let result =
        with_runner_projection_failpoint(RunnerProjectionCheckpoint::LocalConfirmed, || {
            write_projection(&mut store, &primary, Some(&after))
        });
    assert!(result.is_err());
    assert!(read_projection_primary(&path).is_null());
    assert_eq!(
        serde_yaml::from_slice::<serde_yaml::Value>(
            &std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap()
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
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let (primary, before) = projection_values("before");
    let (_, after) = projection_values("after");
    write_projection(&mut store, &primary, Some(&before)).unwrap();
    let old_local = std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap();
    let foreign = b"workPath: foreign-project-work\n";
    let result = with_runner_projection_hook(
        RunnerProjectionCheckpoint::GuardInstalled,
        move |path| {
            std::fs::rename(
                path.join(EFFECTIVE_CONFIG_NAME),
                path.join("owned-guard-retained.yaml"),
            )
            .unwrap();
            std::fs::write(path.join(EFFECTIVE_CONFIG_NAME), foreign).unwrap();
        },
        || write_projection(&mut store, &primary, Some(&after)),
    );
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(path.join(EFFECTIVE_CONFIG_NAME)).unwrap(),
        foreign
    );
    assert!(serde_yaml::from_slice::<serde_yaml::Value>(
        &std::fs::read(path.join("owned-guard-retained.yaml")).unwrap()
    )
    .unwrap()
    .is_null());
    assert_eq!(
        std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap(),
        old_local
    );
}

#[test]
fn runner_project_projection_foreign_local_in_the_gap_is_preserved_with_the_null_guard() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("projection");
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let (primary, before) = projection_values("before");
    let (_, after) = projection_values("after");
    write_projection(&mut store, &primary, Some(&before)).unwrap();
    let previous_local = std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap();
    let foreign = b"infobases: {foreign: {connection: 'File=/foreign/base'}}\n";
    let result = with_runner_projection_hook(
        RunnerProjectionCheckpoint::LocalGap,
        move |path| {
            assert!(read_projection_primary(path).is_null());
            std::fs::write(path.join(LOCAL_CONFIG_NAME), foreign).unwrap();
        },
        || write_projection(&mut store, &primary, Some(&after)),
    );
    assert!(result.is_err());
    assert!(
        read_projection_primary(&path).is_null(),
        "a local refusal must not reinstate a valid primary"
    );
    assert_eq!(
        std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap(),
        foreign
    );
    assert!(
        std::fs::read_dir(&path).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".config-recovery-")
                && std::fs::read(entry.path()).is_ok_and(|bytes| bytes == previous_local)
        }),
        "the old owned local layer must remain in recovery when a foreign file blocks restoration"
    );
}

#[test]
fn runner_project_projection_interrupted_local_removal_is_repaired_at_the_same_owner_path() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("projection");
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let (primary, before) = projection_values("before");
    let (_, after) = projection_values("after");
    write_projection(&mut store, &primary, Some(&before)).unwrap();
    let stable_config = store.config_path().to_path_buf();
    let result = with_runner_projection_failpoint(RunnerProjectionCheckpoint::LocalGap, || {
        write_projection(&mut store, &primary, None)
    });
    assert!(result.is_err());
    assert!(read_projection_primary(&path).is_null());
    assert!(!path.join(LOCAL_CONFIG_NAME).exists());
    drop(store);
    let mut reopened = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    write_projection(&mut reopened, &primary, Some(&after)).unwrap();
    assert_eq!(reopened.config_path(), stable_config);
    assert_eq!(read_projection_primary(&path), primary);
    assert_eq!(
        serde_yaml::from_slice::<serde_yaml::Value>(
            &std::fs::read(path.join(LOCAL_CONFIG_NAME)).unwrap()
        )
        .unwrap(),
        after
    );
}

#[test]
fn runner_project_projection_leaves_an_unchanged_local_file_at_its_retained_identity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("projection");
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let (primary, local) = projection_values("unchanged");
    write_projection(&mut store, &primary, Some(&local)).unwrap();
    let local_path = path.join(LOCAL_CONFIG_NAME);
    let identity = file_identity(&std::fs::File::open(&local_path).unwrap()).unwrap();
    with_runner_projection_hook(
        RunnerProjectionCheckpoint::GuardInstalled,
        |_| panic!("unchanged local must not install an update guard"),
        || write_projection(&mut store, &primary, Some(&local)),
    )
    .unwrap();
    assert_eq!(
        file_identity(&std::fs::File::open(&local_path).unwrap()).unwrap(),
        identity
    );
    assert_eq!(read_projection_primary(&path), primary);
}

#[test]
fn runner_project_store_keeps_a_replaced_config_and_directory_untouched() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("runner-project");
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let value: serde_yaml::Value =
        serde_yaml::from_str("infobases: {origin: {password: secret}}\n").unwrap();
    store.write_config(LOCAL_CONFIG_NAME, &value).unwrap();
    let local = path.join(LOCAL_CONFIG_NAME);
    std::fs::rename(&local, path.join("original-local.yaml")).unwrap();
    std::fs::write(&local, b"foreign replacement").unwrap();
    assert!(store.remove_config(LOCAL_CONFIG_NAME).is_err());
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
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    let before = serde_yaml::Value::String("original secret".into());
    store.write_config(LOCAL_CONFIG_NAME, &before).unwrap();
    let local = path.join(LOCAL_CONFIG_NAME);
    let displaced = path.join("displaced-local.yaml");
    let hook_local = local.clone();
    let hook_displaced = displaced.clone();
    set_before_fixture_config_rename_hook(move || {
        std::fs::rename(&hook_local, &hook_displaced).unwrap();
        std::fs::write(&hook_local, b"foreign publication boundary").unwrap();
    });
    let result = store.write_config(
        LOCAL_CONFIG_NAME,
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
    let mut store = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    for name in [EFFECTIVE_CONFIG_NAME, LOCAL_CONFIG_NAME] {
        let observed = std::rc::Rc::new(std::cell::Cell::new(false));
        let hook_observed = observed.clone();
        with_effective_config_create_hook(
            move |path| {
                let file = std::fs::File::open(path).unwrap();
                assert_eq!(file.metadata().unwrap().len(), 0);
                verify_private_owner(&file).unwrap();
                let directory = open_directory_nofollow(path.parent().unwrap()).unwrap();
                verify_private_owner(&directory).unwrap();
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
        verify_private_owner(&file).unwrap();
    }
}

#[test]
fn runner_project_store_lock_survives_removal_and_does_not_lock_other_workspaces() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("workspace-a");
    let mut owner = RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
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
        with_test_project_lock_contention(
            move || {
                contended_tx.send(()).unwrap();
            },
            || RunnerProjectStore::open_and_lock(&contender_path, &contender_cancellation).err(),
        )
    });
    contended_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("same projection must retain the original lock after removal");
    cancellation.cancel();
    assert!(matches!(
        contender.join().unwrap().unwrap(),
        RunnerProjectLockFailure::Cancelled
    ));

    let independent_cancellation = CancellationToken::new();
    let child_cancellation = independent_cancellation.clone();
    let sibling = root.path().join("workspace-b");
    let (independent_tx, independent_rx) = std::sync::mpsc::channel();
    let independent = std::thread::spawn(move || {
        let acquired = RunnerProjectStore::open_and_lock(&sibling, &child_cancellation);
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
    RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
}
