use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
type Hook = Arc<dyn Fn(bool) + Send + Sync>;
static HOOKS: Mutex<Vec<(PathBuf, Hook)>> = Mutex::new(Vec::new());
pub(crate) struct Guard(PathBuf);
impl Drop for Guard {
    fn drop(&mut self) {
        HOOKS.lock().unwrap().retain(|(root, _)| root != &self.0);
    }
}
pub(crate) fn install(root: &Path, hook: Hook) -> Guard {
    let root = std::fs::canonicalize(root).unwrap();
    let mut hooks = HOOKS.lock().unwrap();
    assert!(
        !hooks.iter().any(|(path, _)| path == &root),
        "duplicate workspace load observer"
    );
    hooks.push((root.clone(), hook));
    Guard(root)
}
pub(super) fn completed(root: &Path, failed: bool) {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    let hook = HOOKS
        .lock()
        .unwrap()
        .iter()
        .find(|(path, _)| path == &root)
        .map(|(_, hook)| Arc::clone(hook));
    // The observer may change a fixture or cancel the real caller. It never
    // replaces the loader result and must run outside the registry lock.
    if let Some(hook) = hook {
        hook(failed);
    }
}
