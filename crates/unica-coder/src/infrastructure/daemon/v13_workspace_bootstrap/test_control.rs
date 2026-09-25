use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

#[derive(Default)]
struct Gate {
    state: Mutex<(usize, bool)>,
    changed: Condvar,
}

fn gates() -> &'static Mutex<HashMap<PathBuf, Arc<Gate>>> {
    static GATES: OnceLock<Mutex<HashMap<PathBuf, Arc<Gate>>>> = OnceLock::new();
    GATES.get_or_init(Mutex::default)
}

pub(crate) struct HealthInspectionPause {
    workspace: PathBuf,
    gate: Arc<Gate>,
}

impl HealthInspectionPause {
    pub(crate) fn install(workspace: PathBuf) -> Self {
        let gate = Arc::new(Gate::default());
        assert!(gates()
            .lock()
            .unwrap()
            .insert(workspace.clone(), Arc::clone(&gate))
            .is_none());
        Self { workspace, gate }
    }

    pub(crate) fn wait_until_entered(&self) {
        let (state, _) = self
            .gate
            .changed
            .wait_timeout_while(
                self.gate.state.lock().unwrap(),
                Duration::from_secs(10),
                |(entries, _)| *entries == 0,
            )
            .unwrap();
        assert_eq!(state.0, 1, "root inspection reached real health traversal");
    }

    pub(crate) fn entries(&self) -> usize {
        self.gate.state.lock().unwrap().0
    }

    pub(crate) fn release(&self) {
        self.gate.state.lock().unwrap().1 = true;
        self.gate.changed.notify_all();
    }
}

impl Drop for HealthInspectionPause {
    fn drop(&mut self) {
        self.release();
        gates().lock().unwrap().remove(&self.workspace);
    }
}

pub(super) fn pause_before_health(workspace: &Path) {
    let gate = gates().lock().unwrap().get(workspace).cloned();
    if let Some(gate) = gate {
        let mut state = gate.state.lock().unwrap();
        state.0 += 1;
        gate.changed.notify_all();
        while !state.1 {
            state = gate.changed.wait(state).unwrap();
        }
    }
}
