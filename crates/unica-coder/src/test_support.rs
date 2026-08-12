use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn process_cwd_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn canonical_path(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).expect("test path identity must canonicalize");
    if std::path::MAIN_SEPARATOR == '\\' {
        let display = canonical.to_string_lossy();
        if let Some(path) = display.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{path}"));
        }
        if let Some(path) = display.strip_prefix(r"\\?\") {
            return PathBuf::from(path);
        }
    }
    canonical
}

pub(crate) struct ProcessCwdGuard {
    previous: PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl ProcessCwdGuard {
    pub(crate) fn enter(directory: &Path) -> Result<Self, String> {
        let lock = process_cwd_lock();
        let previous = std::env::current_dir().map_err(|error| error.to_string())?;
        std::env::set_current_dir(directory).map_err(|error| error.to_string())?;
        Ok(Self {
            previous,
            _lock: lock,
        })
    }
}

impl Drop for ProcessCwdGuard {
    fn drop(&mut self) {
        if let Err(error) = std::env::set_current_dir(&self.previous) {
            if !std::thread::panicking() {
                panic!("test current directory must be restorable: {error}");
            }
        }
    }
}

pub(crate) fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = std::fs::read_dir(current)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                visit(root, &path, output);
            } else if metadata.is_file() {
                output.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    visit(root, root, &mut snapshot);
    snapshot
}
