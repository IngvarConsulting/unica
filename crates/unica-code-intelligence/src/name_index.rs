//! Which modules of a source set spell a name. The index answers the first
//! question of every cross-module search — where a name can occur at all — so
//! a search parses only the modules that may refer to it, not the corpus.
//!
//! Names are keyed without letter case, as BSL compares them. A module is
//! keyed by its source-root-relative path and carries a stamp the owner
//! compares to tell a changed module from an unchanged one.

use std::collections::{HashMap, HashSet};

pub use unica_bsl_parser::symbols::name_key;

/// A replaced or removed module leaves a dead slot behind; past this share of
/// dead slots the postings are rebuilt without them.
const DEAD_SHARE_BEFORE_COMPACTION: usize = 4;

struct Module<S> {
    path: Box<str>,
    stamp: S,
}

pub struct NameIndex<S> {
    /// Module slots; a replaced or removed module's slot is `None`.
    slots: Vec<Option<Module<S>>>,
    by_path: HashMap<Box<str>, u32>,
    /// Name key to the slots of the modules that spell it, dead ones included.
    postings: HashMap<Box<str>, Vec<u32>>,
    dead: usize,
}

impl<S> Default for NameIndex<S> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            by_path: HashMap::new(),
            postings: HashMap::new(),
            dead: 0,
        }
    }
}

impl<S> NameIndex<S> {
    /// The stamp the module at `path` was indexed with.
    pub fn stamp(&self, path: &str) -> Option<&S> {
        let slot = *self.by_path.get(path)?;
        self.slots[slot as usize]
            .as_ref()
            .map(|module| &module.stamp)
    }

    /// Every indexed module path.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.by_path.keys().map(AsRef::as_ref)
    }

    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// Indexes the module at `path` with the name keys its text spells,
    /// replacing what was indexed for it before.
    pub fn insert(&mut self, path: &str, stamp: S, keys: HashSet<String>) {
        self.remove(path);
        let slot = u32::try_from(self.slots.len()).expect("fewer than 2^32 module slots");
        self.slots.push(Some(Module {
            path: path.into(),
            stamp,
        }));
        self.by_path.insert(path.into(), slot);
        for key in keys {
            self.postings.entry(key.into()).or_default().push(slot);
        }
    }

    pub fn remove(&mut self, path: &str) {
        if let Some(slot) = self.by_path.remove(path) {
            self.slots[slot as usize] = None;
            self.dead += 1;
            if self.dead * DEAD_SHARE_BEFORE_COMPACTION > self.slots.len() {
                self.compact();
            }
        }
    }

    /// The paths of the modules that spell `name`, in any letter case.
    pub fn modules_naming(&self, name: &str) -> Vec<&str> {
        self.postings
            .get(name_key(name).as_str())
            .into_iter()
            .flatten()
            .filter_map(|slot| self.slots[*slot as usize].as_ref())
            .map(|module| module.path.as_ref())
            .collect()
    }

    /// Renumbers the live slots and drops the dead ones from every posting.
    fn compact(&mut self) {
        let mut renumbered = vec![None; self.slots.len()];
        let mut slots = Vec::with_capacity(self.by_path.len());
        for (old, module) in std::mem::take(&mut self.slots).into_iter().enumerate() {
            if let Some(module) = module {
                renumbered[old] = Some(slots.len() as u32);
                slots.push(Some(module));
            }
        }
        self.postings.retain(|_, posting| {
            posting.retain_mut(|slot| match renumbered[*slot as usize] {
                Some(new) => {
                    *slot = new;
                    true
                }
                None => false,
            });
            posting.shrink_to_fit();
            !posting.is_empty()
        });
        for slot in self.by_path.values_mut() {
            *slot = renumbered[*slot as usize].expect("an indexed path has a live slot");
        }
        self.slots = slots;
        self.dead = 0;
    }

    /// An estimate of the heap the index holds, for its resource account.
    pub fn approximate_bytes(&self) -> usize {
        let word = std::mem::size_of::<usize>();
        let postings: usize = self
            .postings
            .iter()
            .map(|(key, slots)| key.len() + 2 * word + 4 * slots.capacity() + 3 * word)
            .sum();
        let modules: usize = self
            .by_path
            .keys()
            .map(|path| 2 * path.len() + 6 * word + std::mem::size_of::<S>())
            .sum();
        postings + modules
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name_key(name)).collect()
    }

    #[test]
    fn a_name_is_found_in_any_case_and_follows_module_changes() {
        let mut index = NameIndex::default();
        index.insert("A.bsl", 1, keys(&["ОбщийМодуль", "Справочники"]));
        index.insert("B.bsl", 1, keys(&["общиймодуль"]));
        let mut found = index.modules_naming("ОБЩИЙМОДУЛЬ");
        found.sort_unstable();
        assert_eq!(found, ["A.bsl", "B.bsl"]);

        index.insert("A.bsl", 2, keys(&["Справочники"]));
        assert_eq!(index.modules_naming("ОбщийМодуль"), ["B.bsl"]);
        assert_eq!(index.stamp("A.bsl"), Some(&2));
        index.remove("B.bsl");
        assert!(index.modules_naming("ОбщийМодуль").is_empty());
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn compaction_keeps_every_live_module_findable() {
        let mut index = NameIndex::default();
        for round in 0..20 {
            for module in 0..10 {
                let name = format!("Имя{}", module % 3);
                index.insert(&format!("{module}.bsl"), round, keys(&[&name]));
            }
        }
        assert!(index.slots.len() < 40, "dead slots were compacted");
        let mut found = index.modules_naming("имя0");
        found.sort_unstable();
        assert_eq!(found, ["0.bsl", "3.bsl", "6.bsl", "9.bsl"]);
        assert!(index.approximate_bytes() > 0);
    }
}
