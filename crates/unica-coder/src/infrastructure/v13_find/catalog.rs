//! Каталог имён одного набора исходников: то, что прошлый обход раскладки
//! прочитал, с штампами прочитанного.
//!
//! Актор держит каталог между вызовами рядом с остальными артефактами набора
//! (см. [`RegistrationCache`](crate::infrastructure::v13_large_configuration::RegistrationCache)).
//! Запись годна, пока штамп её каталога или файла (физическая идентичность,
//! длина, время изменения содержимого и метаданных) совпадает с текущим:
//! изменённое читается заново, остальное берётся из каталога. Штамп моложе
//! окна не сохраняется, и такой файл читается при каждом обходе. Пропуск
//! чтения не сохраняется, поэтому повторный обход читает его снова.
//! Собственная публикация `apply` сбрасывает затронутую коллекцию.

use crate::infrastructure::v13_large_configuration::SETTLE_WINDOW;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};
use unica_source_contract::events::DomainEvent;
use unica_source_contract::read::{FileIdentity, FileStamp};
use unica_source_designer::layout::{metadata_kind, metadata_kind_by_directory};

pub(crate) struct NameCatalog {
    slot: Mutex<Slot>,
    settle_window: Duration,
}

#[derive(Default)]
struct Slot {
    /// Advances on every reset, so a walk that started before it does not
    /// return what it read.
    generation: u64,
    layout: Option<CatalogLayout>,
}

/// What one walk read below one physical root, keyed by the path relative
/// to that root.
#[derive(Default)]
pub(super) struct CatalogLayout {
    root: Option<FileIdentity>,
    /// Numbers the walks; each kept item notes the last walk that met it.
    walk: u64,
    listings: HashMap<PathBuf, (FileStamp, Arc<[OsString]>, u64)>,
    entries: HashMap<PathBuf, (CachedEntry, u64)>,
}

#[derive(Clone)]
pub(super) enum CachedEntry {
    /// The probed name was absent while its directory had this stamp.
    Absent(FileStamp),
    Directory(FileStamp),
    /// A regular file the walk does not read.
    File(FileStamp),
    Descriptor(FileStamp, Arc<DescriptorFacts>),
}

/// The identity facts of one descriptor head.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct DescriptorFacts {
    pub(super) name: Option<String>,
    pub(super) synonym: Option<String>,
    /// Whether the head declares the owner element and name its place in
    /// the layout requires.
    pub(super) declares_owner: bool,
}

/// One walk's private copy of the catalog. It goes back only if no reset
/// happened meanwhile.
pub(super) struct CatalogCheckout<'c> {
    catalog: &'c NameCatalog,
    generation: u64,
    now: SystemTime,
    pub(super) layout: CatalogLayout,
}

impl Default for NameCatalog {
    fn default() -> Self {
        Self {
            slot: Mutex::new(Slot::default()),
            settle_window: SETTLE_WINDOW,
        }
    }
}

impl NameCatalog {
    #[cfg(test)]
    pub(crate) fn with_settle_window_for_test(window: Duration) -> Self {
        Self {
            slot: Mutex::new(Slot::default()),
            settle_window: window,
        }
    }

    /// How many listings and entries the catalog keeps.
    #[cfg(test)]
    pub(crate) fn kept_paths_for_test(&self) -> usize {
        self.slot()
            .layout
            .as_ref()
            .map_or(0, |layout| layout.listings.len() + layout.entries.len())
    }

    /// A poisoned slot is emptied and reused.
    fn slot(&self) -> MutexGuard<'_, Slot> {
        match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                guard.layout = None;
                guard.generation = guard.generation.wrapping_add(1);
                self.slot.clear_poison();
                guard
            }
        }
    }

    /// Takes the retained layout of `root` for one walk. Concurrent walks
    /// each get their own copy: the one that finds the slot empty starts
    /// from nothing.
    pub(super) fn checkout(&self, root: FileIdentity) -> CatalogCheckout<'_> {
        let mut slot = self.slot();
        let mut layout = slot
            .layout
            .take()
            .filter(|layout| layout.root == Some(root))
            .unwrap_or_else(|| CatalogLayout {
                root: Some(root),
                ..CatalogLayout::default()
            });
        layout.walk = layout.walk.wrapping_add(1);
        CatalogCheckout {
            catalog: self,
            generation: slot.generation,
            now: SystemTime::now(),
            layout,
        }
    }

    /// Drops what a committed publication changed in `source_set`: the
    /// collections of the kinds its events name, or everything when an event
    /// names no collection. An event without a source-set prefix belongs to
    /// `default_source_set`, the one the publication targeted.
    pub(crate) fn forget_published(
        &self,
        source_set: &str,
        default_source_set: &str,
        events: &[DomainEvent],
    ) {
        let events = events
            .iter()
            .filter(|event| published_source_set(event, default_source_set) == source_set)
            .collect::<Vec<_>>();
        if events.is_empty() {
            return;
        }
        let mut slot = self.slot();
        slot.generation = slot.generation.wrapping_add(1);
        let Some(layout) = slot.layout.as_mut() else {
            return;
        };
        let mut kinds = Vec::with_capacity(events.len());
        for event in events {
            let Some(tag) = published_kind(event) else {
                slot.layout = None;
                return;
            };
            kinds.push(tag);
        }
        let affected = |relative: &Path| {
            match relative.components().next() {
                Some(Component::Normal(directory)) => directory
                    .to_str()
                    .and_then(metadata_kind_by_directory)
                    .is_some_and(|layout| kinds.contains(&layout.tag)),
                // The root listing names the collections themselves.
                _ => true,
            }
        };
        layout.listings.retain(|relative, _| !affected(relative));
        layout.entries.retain(|relative, _| !affected(relative));
    }
}

fn published_source_set<'e>(event: &'e DomainEvent, default_source_set: &'e str) -> &'e str {
    match &event.details {
        Some(details) => &details.source_set,
        None => event
            .artifact
            .split_once(':')
            .map_or(default_source_set, |(source_set, _)| source_set),
    }
}

/// The top-level metadata kind an event changed, when it names one.
fn published_kind(event: &DomainEvent) -> Option<&'static str> {
    let artifact = event
        .details
        .as_ref()
        .map_or(event.artifact.as_str(), |details| details.owner.as_str());
    let local = artifact
        .split_once(':')
        .map_or(artifact, |(_, local)| local);
    let tag = local.split('.').next()?;
    metadata_kind(tag).map(|layout| layout.tag)
}

impl CatalogCheckout<'_> {
    /// Whether a stamp is old enough to keep what was read under it.
    pub(super) fn retains(&self, stamp: &FileStamp) -> bool {
        stamp.settled(self.now, self.catalog.settle_window)
    }

    pub(super) fn listing(
        &mut self,
        relative: &Path,
        stamp: &FileStamp,
    ) -> Option<Arc<[OsString]>> {
        let walk = self.layout.walk;
        let (kept, names, seen) = self.layout.listings.get_mut(relative)?;
        if kept != stamp {
            return None;
        }
        *seen = walk;
        Some(Arc::clone(names))
    }

    pub(super) fn keep_listing(
        &mut self,
        relative: &Path,
        stamp: FileStamp,
        names: &Arc<[OsString]>,
    ) {
        if self.retains(&stamp) {
            self.layout.listings.insert(
                relative.to_path_buf(),
                (stamp, Arc::clone(names), self.layout.walk),
            );
        } else {
            self.layout.listings.remove(relative);
        }
    }

    pub(super) fn entry(&mut self, relative: &Path) -> Option<CachedEntry> {
        let walk = self.layout.walk;
        let (entry, seen) = self.layout.entries.get_mut(relative)?;
        *seen = walk;
        Some(entry.clone())
    }

    /// Keeps an entry whose deciding stamp has settled; otherwise forgets
    /// what was kept for that path.
    pub(super) fn keep_entry(&mut self, relative: PathBuf, entry: CachedEntry) {
        let stamp = match &entry {
            CachedEntry::Absent(stamp)
            | CachedEntry::Directory(stamp)
            | CachedEntry::File(stamp)
            | CachedEntry::Descriptor(stamp, _) => stamp,
        };
        if self.retains(stamp) {
            let walk = self.layout.walk;
            self.layout.entries.insert(relative, (entry, walk));
        } else {
            self.layout.entries.remove(&relative);
        }
    }

    pub(super) fn forget_entry(&mut self, relative: &Path) {
        self.layout.entries.remove(relative);
    }

    /// Returns the layout unless a reset happened since the checkout. A
    /// whole walk met every path the layout still has: what it did not meet
    /// is gone and is dropped.
    pub(super) fn checkin(mut self, whole: bool) {
        if whole {
            let walk = self.layout.walk;
            self.layout.listings.retain(|_, (_, _, seen)| *seen == walk);
            self.layout.entries.retain(|_, (_, seen)| *seen == walk);
        }
        let mut slot = self.catalog.slot();
        if slot.generation == self.generation {
            slot.layout = Some(self.layout);
        }
    }
}
