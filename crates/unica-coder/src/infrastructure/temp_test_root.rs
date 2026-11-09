//! Временный корень теста, который удаляется вместе с тестом.
//!
//! Тест получает путь внутри корня, а корень уходит, когда страж выходит из
//! области видимости: и после успеха, и при панике теста. Корень, который не
//! удалился, называется в выводе теста, а не остаётся в TEMP молча.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct TempTestRoot {
    root: PathBuf,
    path: PathBuf,
}

impl TempTestRoot {
    /// Новый пустой корень `<TEMP>/<prefix>-<nanos>`; путь теста — `relative` внутри него.
    pub(crate) fn new(prefix: &str, relative: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        let path = if relative.is_empty() {
            root.clone()
        } else {
            root.join(relative)
        };
        std::fs::create_dir_all(&path).unwrap();
        Self { root, path }
    }
}

impl Deref for TempTestRoot {
    type Target = PathBuf;

    fn deref(&self) -> &PathBuf {
        &self.path
    }
}

impl AsRef<Path> for TempTestRoot {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempTestRoot {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "временный корень теста не удалён: {}: {error}",
                    self.root.display()
                );
            }
        }
    }
}
