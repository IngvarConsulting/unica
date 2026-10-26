//! The name index of one source set, kept by the workspace actor with the
//! set's other retained artifacts. It is built in the background on first
//! use and refreshed by file stamps, so edits made outside Unica reach it
//! too. Every search that asks where a name can occur uses it: the editor's
//! references now, the removal planner later.
//!
//! The build gives way to the agent: while an MCP call runs it waits, and it
//! stops as soon as the actor lets the set's artifacts go.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant, SystemTime};

use unica_code_intelligence::name_index::NameIndex;
use unica_code_intelligence::symbols::NamedModules;
use unica_source_contract::read::{FileStamp, SourceDirectory, SourceEntry};

use crate::infrastructure::v13_large_configuration::SETTLE_WINDOW;

/// A refresh walk starts again only this long after the last one ended.
pub(crate) const NAME_INDEX_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
/// While an MCP call runs, the build checks again after this long.
pub(crate) const NAME_INDEX_YIELD_POLL: Duration = Duration::from_millis(20);
/// The largest module text the index reads; a larger module is left out.
pub(crate) const NAME_INDEX_MODULE_BYTES: usize = 64 * 1024 * 1024;
/// The heap the index may grow to; past it, modules not yet indexed are left
/// out and every answer says so. Measured on УТ 11: about 38 MiB.
pub(crate) const NAME_INDEX_BUDGET_BYTES: usize = 256 * 1024 * 1024;

static FOREGROUND_CALLS: AtomicUsize = AtomicUsize::new(0);

/// Held while an MCP call executes; the name index build waits meanwhile.
pub(crate) struct ForegroundCall(());

impl ForegroundCall {
    pub(crate) fn begin() -> Self {
        FOREGROUND_CALLS.fetch_add(1, Ordering::AcqRel);
        Self(())
    }
}

impl Drop for ForegroundCall {
    fn drop(&mut self) {
        FOREGROUND_CALLS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn foreground_calls() -> usize {
    FOREGROUND_CALLS.load(Ordering::Acquire)
}

/// Which source-root-relative paths are modules of the set.
pub(crate) type ModulePaths = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

#[derive(Default)]
struct BuildState {
    /// A full walk has finished at least once.
    ready: bool,
    /// The last walk left modules out: the index reached its budget.
    capped: bool,
    running: bool,
    last_end: Option<Instant>,
}

pub(crate) struct SourceNameIndex {
    /// `None` stamps are unsettled: the next walk reads the module again.
    index: RwLock<NameIndex<Option<FileStamp>>>,
    state: Mutex<BuildState>,
    budget: usize,
}

impl Default for SourceNameIndex {
    fn default() -> Self {
        Self {
            index: RwLock::default(),
            state: Mutex::default(),
            budget: NAME_INDEX_BUDGET_BYTES,
        }
    }
}

impl SourceNameIndex {
    /// The paths of the modules that spell `name`, or `None` until the first
    /// build has finished. Starts that build, or a refresh when the last walk
    /// is older than [`NAME_INDEX_REFRESH_INTERVAL`].
    pub(crate) fn modules_naming(
        self: &Arc<Self>,
        name: &str,
        root: &SourceDirectory,
        modules: &ModulePaths,
    ) -> Option<NamedModules> {
        let (ready, capped) = self.refresh(root, modules);
        ready.then(|| NamedModules {
            paths: self
                .index
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .modules_naming(name)
                .into_iter()
                .map(str::to_string)
                .collect(),
            complete: !capped,
        })
    }

    /// Starts a background walk unless one runs or the last one is recent;
    /// answers whether the first build has finished and whether the last
    /// walk left modules out.
    fn refresh(self: &Arc<Self>, root: &SourceDirectory, modules: &ModulePaths) -> (bool, bool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let due = state
            .last_end
            .is_none_or(|end| end.elapsed() >= NAME_INDEX_REFRESH_INTERVAL);
        if !state.running && due {
            state.running = true;
            let owner = Arc::downgrade(self);
            let root = root.clone();
            let modules = Arc::clone(modules);
            std::thread::spawn(move || walk(&owner, &root, &modules));
        }
        (state.ready, state.capped)
    }

    fn finish(&self, complete: bool, capped: bool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.running = false;
        state.ready |= complete;
        if complete {
            state.capped = capped;
        }
        state.last_end = Some(Instant::now());
    }

    /// An estimate of the heap the index holds.
    #[cfg(test)]
    pub(crate) fn approximate_bytes(&self) -> usize {
        self.index
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .approximate_bytes()
    }

    #[cfg(test)]
    pub(crate) fn wait_ready_for_test(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let state = self.state.lock().unwrap();
            if state.ready && !state.running {
                return true;
            }
            drop(state);
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Why a walk stopped before its end.
enum Stop {
    /// The actor let the set's artifacts go.
    Released,
    /// The source root could not be listed.
    Unreadable,
}

/// One walk over the source root: modules whose stamp changed are read and
/// indexed again, and modules no longer there are dropped.
fn walk(owner: &Weak<SourceNameIndex>, root: &SourceDirectory, modules: &ModulePaths) {
    let mut seen = std::collections::HashSet::new();
    let mut capped = false;
    let outcome = visit(owner, root, modules, &mut seen, &mut capped);
    let Some(index) = owner.upgrade() else {
        return;
    };
    if outcome.is_ok() {
        let mut names = index
            .index
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let gone: Vec<String> = names
            .paths()
            .filter(|path| !seen.contains(*path))
            .map(str::to_string)
            .collect();
        for path in gone {
            names.remove(&path);
        }
    }
    index.finish(outcome.is_ok(), capped);
}

fn visit(
    owner: &Weak<SourceNameIndex>,
    root: &SourceDirectory,
    modules: &ModulePaths,
    seen: &mut std::collections::HashSet<String>,
    capped: &mut bool,
) -> Result<(), Stop> {
    let mut stack = vec![(PathBuf::new(), root.clone())];
    while let Some((relative, directory)) = stack.pop() {
        let mut names = Vec::new();
        directory
            .visit_immediate_names(|name| {
                names.push(name);
                Ok::<_, std::io::Error>(())
            })
            .map_err(|_| Stop::Unreadable)?;
        names.sort();
        for name in names {
            let path = relative.join(&name);
            match directory.retain_immediate_child_nofollow(&name) {
                Ok(SourceEntry::Directory(child)) => stack.push((path, child)),
                Ok(SourceEntry::RegularFile(_)) if modules(&path) => {
                    give_way(owner)?;
                    let key = path.to_string_lossy().replace('\\', "/");
                    if index_module(owner, root, &path, &key)? {
                        seen.insert(key);
                    } else {
                        *capped = true;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Waits while an MCP call runs; stops when the index is no longer kept.
fn give_way(owner: &Weak<SourceNameIndex>) -> Result<(), Stop> {
    loop {
        if owner.strong_count() == 0 {
            return Err(Stop::Released);
        }
        if foreground_calls() == 0 {
            return Ok(());
        }
        std::thread::sleep(NAME_INDEX_YIELD_POLL);
    }
}

fn index_module(
    owner: &Weak<SourceNameIndex>,
    root: &SourceDirectory,
    path: &Path,
    key: &str,
) -> Result<bool, Stop> {
    let index = owner.upgrade().ok_or(Stop::Released)?;
    let stamp = root
        .relative_file_stamp(path)
        .ok()
        .flatten()
        // A stamp younger than the window may hide a change in the same tick:
        // such a module is read again by the next refresh.
        .filter(|stamp| stamp.settled(SystemTime::now(), SETTLE_WINDOW));
    let unchanged = stamp.is_some()
        && index
            .index
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stamp(key)
            == Some(&stamp);
    if unchanged {
        return Ok(true);
    }
    let over_budget = {
        let names = index
            .index
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        names.stamp(key).is_none() && names.approximate_bytes() >= index.budget
    };
    if over_budget {
        return Ok(false);
    }
    let keys = root
        .read_relative_regular_bounded(path, NAME_INDEX_MODULE_BYTES)
        .ok()
        .map(|bytes| {
            let text = String::from_utf8_lossy(&bytes);
            unica_bsl_parser::parse(text.trim_start_matches('\u{feff}')).name_keys()
        });
    let mut names = index
        .index
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match keys {
        Some(keys) => names.insert(key, stamp, keys),
        None => names.remove(key),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modules() -> ModulePaths {
        Arc::new(|path: &Path| path.extension().is_some_and(|extension| extension == "bsl"))
    }

    fn root(path: &Path) -> SourceDirectory {
        unica_workspace::read::open_source_directory(&path.canonicalize().unwrap())
            .expect("the test root opens")
    }

    #[test]
    fn the_index_is_built_in_the_background_and_follows_file_changes() {
        let directory = tempfile::tempdir().unwrap();
        let module = |relative: &str, text: &str| {
            let path = directory.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        module(
            "CommonModules/А/Ext/Module.bsl",
            "Процедура П() Экспорт\n\tОБЩИЙ.Сделать();\nКонецПроцедуры\n",
        );
        module("CommonModules/Б/Ext/Module.bsl", "// Сделать\n");
        let root = root(directory.path());
        let index = Arc::new(SourceNameIndex::default());

        assert_eq!(index.modules_naming("сделать", &root, &modules()), None);
        assert!(index.wait_ready_for_test(Duration::from_secs(10)));
        assert_eq!(
            index.modules_naming("сделать", &root, &modules()),
            Some(NamedModules {
                paths: vec!["CommonModules/А/Ext/Module.bsl".to_string()],
                complete: true
            })
        );

        std::fs::remove_file(directory.path().join("CommonModules/А/Ext/Module.bsl")).unwrap();
        module("CommonModules/Б/Ext/Module.bsl", "Общий.СДЕЛАТЬ();\n");
        index.state.lock().unwrap().last_end = None;
        index.modules_naming("сделать", &root, &modules());
        assert!(index.wait_ready_for_test(Duration::from_secs(10)));
        assert_eq!(
            index.modules_naming("Сделать", &root, &modules()),
            Some(NamedModules {
                paths: vec!["CommonModules/Б/Ext/Module.bsl".to_string()],
                complete: true
            })
        );
        assert!(index.approximate_bytes() > 0);
    }

    #[test]
    fn an_index_past_its_budget_answers_incomplete_rather_than_refusing() {
        let directory = tempfile::tempdir().unwrap();
        for module in ["А", "Б", "В"] {
            std::fs::write(
                directory.path().join(format!("{module}.bsl")),
                "Сделать();\n",
            )
            .unwrap();
        }
        let root = root(directory.path());
        let index = Arc::new(SourceNameIndex {
            budget: 1,
            ..SourceNameIndex::default()
        });
        index.modules_naming("сделать", &root, &modules());
        assert!(index.wait_ready_for_test(Duration::from_secs(10)));
        let answer = index.modules_naming("сделать", &root, &modules()).unwrap();
        assert_eq!(answer.paths.len(), 1);
        assert!(!answer.complete);
    }

    #[test]
    fn the_build_waits_while_an_mcp_call_runs() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("М.bsl"), "Сделать();\n").unwrap();
        let root = root(directory.path());
        let index = Arc::new(SourceNameIndex::default());
        let call = ForegroundCall::begin();
        index.modules_naming("сделать", &root, &modules());
        assert!(!index.wait_ready_for_test(Duration::from_millis(300)));
        drop(call);
        assert!(index.wait_ready_for_test(Duration::from_secs(10)));
    }
}
