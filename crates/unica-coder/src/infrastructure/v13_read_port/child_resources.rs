//! Проверка дочерних ресурсов объекта в `view`: та же, что в `check`, но
//! через удержанный корень набора. Итог хранится между вызовами под
//! ревизией каталогов форм, макетов и команд объекта.
use super::{ProviderReadAuthority, ViewError};
use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use unica_address::source_target::MetadataAddress;
use unica_metadata::child_resources::{TypedChildEntryKind, TypedChildResourceRead};
use unica_metadata::MetaDiagnostic;
use unica_refusal::RefusalDetail;
use unica_source_contract::read::{FileStamp, SourceDirectory, SourceEntry};

fn codec() -> &'static dyn unica_metadata::read_codec::MetadataReadCodec {
    crate::infrastructure::family_codecs::metadata_read_codec()
}

/// Один вход каталогов дочерних ресурсов.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RevisionEntry {
    Absent,
    Directory,
    File(FileStamp),
    Link,
    Other,
}

/// Ревизия того, что читает проверка дочерних ресурсов: описание владельца
/// и каждый вход каталогов форм, макетов и команд со штампом файла.
/// Содержимое файлов не читается: изменение файла меняет его штамп,
/// а добавление и удаление — список входов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildResourceRevision {
    owner: [u8; 32],
    entries: Vec<(PathBuf, RevisionEntry)>,
}

impl ChildResourceRevision {
    pub(crate) fn file_stamps(&self) -> impl Iterator<Item = &FileStamp> {
        self.entries.iter().filter_map(|(_, entry)| match entry {
            RevisionEntry::File(stamp) => Some(stamp),
            _ => None,
        })
    }
}

impl ProviderReadAuthority {
    /// Ошибки форм, макетов и команд объекта — те же, что называет `check`.
    /// Ревизия снимается до проверки, поэтому изменение во время проверки
    /// не совпадёт с ней при следующем чтении. Отмена и срок отказывают
    /// чтению, а не становятся ошибкой ресурса.
    pub(crate) fn child_resource_diagnostics(
        &self,
        target: &MetadataAddress,
        kind: &str,
        name: &str,
        descriptor: &[u8],
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Vec<MetaDiagnostic>, ViewError> {
        let relative = self.metadata_descriptor_relative(target)?;
        let owner_xml = std::str::from_utf8(descriptor).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "metadata descriptor is not UTF-8",
            )
        })?;
        let revision = child_resource_revision(&self.root, &relative, descriptor, checkpoint)?;
        if let Some(diagnostics) = revision
            .as_ref()
            .and_then(|revision| self.artifacts.retained_child_resources(&relative, revision))
        {
            return Ok(diagnostics.as_ref().clone());
        }
        let reader = RetainedChildResources::new(&self.root, checkpoint);
        let diagnostics =
            codec().child_resource_diagnostics(&reader, &relative, target, kind, name, owner_xml);
        reader.finish()?;
        if let Some(revision) = revision {
            self.artifacts.retain_child_resources(
                relative,
                revision,
                Arc::new(diagnostics.clone()),
            );
        }
        Ok(diagnostics)
    }
}

/// `None` — ревизию снять не удалось; проверка тогда идёт без кеша и сама
/// называет непрочитанное.
fn child_resource_revision(
    root: &SourceDirectory,
    descriptor: &Path,
    owner: &[u8],
    checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
) -> Result<Option<ChildResourceRevision>, ViewError> {
    use sha2::Digest;

    let mut interrupted = None;
    let mut check = || {
        checkpoint().map_err(|error| {
            interrupted = Some(error);
            io::Error::other("child resource revision interrupted")
        })
    };
    let mut entries = Vec::new();
    let walked = (|| -> io::Result<()> {
        let object = match retained_directory(root, &descriptor.with_extension("")) {
            Ok(directory) => Some(directory),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        for name in codec().observed_child_directories() {
            let directory = match object
                .as_ref()
                .map(|object| object.retain_immediate_child_nofollow(std::ffi::OsStr::new(name)))
            {
                None => Err(io::ErrorKind::NotFound.into()),
                Some(entry) => entry,
            };
            match directory {
                Ok(SourceEntry::Directory(directory)) => {
                    check()?;
                    entries.push((PathBuf::from(name), RevisionEntry::Directory));
                    walk_in_parallel(&directory, PathBuf::from(name), &mut entries, &mut check)?;
                    check()?;
                }
                Ok(entry) => entries.push((PathBuf::from(name), entry_kind(entry)?)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    entries.push((PathBuf::from(name), RevisionEntry::Absent));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    })();
    if let Some(error) = interrupted {
        return Err(error);
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(walked.ok().map(|()| ChildResourceRevision {
        owner: sha2::Sha256::digest(owner).into(),
        entries,
    }))
}

/// The entries of one child directory, its names spread over the host's
/// cores: on Windows every entry is a handle to open, and a form directory
/// of a large object has hundreds. The workers only stamp, never read
/// content. The calling thread takes a share itself and runs the caller's
/// checkpoint between its entries; an interruption stops the other workers
/// between theirs. A tree deeper than the observation's own bound is not
/// walked: the observation names it and nothing is retained.
fn walk_in_parallel(
    directory: &SourceDirectory,
    relative: PathBuf,
    entries: &mut Vec<(PathBuf, RevisionEntry)>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};

    let names = directory.read_immediate_names_with_limit(None, &mut *check)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .clamp(1, names.len().max(1));
    let mut shares = vec![Vec::new(); workers];
    for (index, name) in names.into_iter().enumerate() {
        shares[index % workers].push(name);
    }
    let own = shares.remove(0);
    let stop = AtomicBool::new(false);
    let (walked, checked) = std::thread::scope(|scope| {
        let relative = &relative;
        let stop = &stop;
        let handles = shares
            .into_iter()
            .map(|share| {
                scope.spawn(move || -> io::Result<Vec<(PathBuf, RevisionEntry)>> {
                    let mut stopped = || {
                        if stop.load(Ordering::Relaxed) {
                            Err(io::Error::other("child resource revision stopped"))
                        } else {
                            Ok(())
                        }
                    };
                    let mut found = Vec::new();
                    for name in share {
                        stopped()?;
                        visit(directory, relative, &name, 1, &mut found, &mut stopped)?;
                    }
                    Ok(found)
                })
            })
            .collect::<Vec<_>>();
        let mut found = Vec::new();
        let mut interruptible = || check().inspect_err(|_| stop.store(true, Ordering::Relaxed));
        let checked = own.iter().try_for_each(|name| {
            interruptible()?;
            visit(directory, relative, name, 1, &mut found, &mut interruptible)
        });
        if checked.is_err() {
            stop.store(true, Ordering::Relaxed);
        }
        let mut walked = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect::<Vec<_>>();
        walked.push(Ok(found));
        (walked, checked)
    });
    checked?;
    for found in walked {
        entries.extend(found?);
    }
    Ok(())
}

fn visit(
    directory: &SourceDirectory,
    relative: &Path,
    name: &std::ffi::OsStr,
    depth: usize,
    entries: &mut Vec<(PathBuf, RevisionEntry)>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<()> {
    if depth > codec().child_tree_max_depth() {
        return Err(io::Error::other(
            "child resource tree is deeper than observed",
        ));
    }
    let path = relative.join(name);
    match directory.retain_immediate_child_nofollow(name)? {
        SourceEntry::Directory(child) => {
            entries.push((path.clone(), RevisionEntry::Directory));
            for name in child.read_immediate_names_with_limit(None, &mut *check)? {
                check()?;
                visit(&child, &path, &name, depth + 1, entries, check)?;
            }
            Ok(())
        }
        entry => {
            entries.push((path, entry_kind(entry)?));
            Ok(())
        }
    }
}

fn entry_kind(entry: SourceEntry) -> io::Result<RevisionEntry> {
    Ok(match entry {
        SourceEntry::Directory(_) => RevisionEntry::Directory,
        SourceEntry::RegularFile(file) => RevisionEntry::File(file.try_clone_file()?.stamp()?),
        SourceEntry::ReparsePoint => RevisionEntry::Link,
        SourceEntry::Unsupported => RevisionEntry::Other,
    })
}

/// A directory under the retained root, opened one name at a time without
/// following links.
fn retained_directory(root: &SourceDirectory, relative: &Path) -> io::Result<SourceDirectory> {
    let mut directory = root.clone();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "child resource path is not relative to the source root",
            ));
        };
        directory = directory.retain_directory_child(name)?;
    }
    Ok(directory)
}

/// The observation's reads through the retained root. Its paths are relative
/// to that root and are never reopened through the ambient filesystem.
struct RetainedChildResources<'a> {
    root: &'a SourceDirectory,
    checkpoint: RefCell<&'a mut dyn FnMut() -> Result<(), ViewError>>,
    interrupted: RefCell<Option<ViewError>>,
}

impl<'a> RetainedChildResources<'a> {
    fn new(
        root: &'a SourceDirectory,
        checkpoint: &'a mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Self {
        Self {
            root,
            checkpoint: RefCell::new(checkpoint),
            interrupted: RefCell::new(None),
        }
    }

    fn check(&self) -> io::Result<()> {
        if self.interrupted.borrow().is_none() {
            if let Err(error) = (self.checkpoint.borrow_mut())() {
                *self.interrupted.borrow_mut() = Some(error);
            }
        }
        if self.interrupted.borrow().is_some() {
            return Err(io::Error::other("child resource read interrupted"));
        }
        Ok(())
    }

    /// The interruption that stopped the observation, if any.
    fn finish(self) -> Result<(), ViewError> {
        match self.interrupted.into_inner() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl TypedChildResourceRead for RetainedChildResources<'_> {
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.check()?;
        let mut bytes = Vec::new();
        self.root.visit_relative_regular_chunks(
            path,
            || self.check(),
            |chunk| {
                bytes.try_reserve(chunk.len()).map_err(io::Error::other)?;
                bytes.extend_from_slice(chunk);
                Ok(())
            },
        )?;
        Ok(bytes)
    }

    /// Only a proven absence is `false`: an entry that cannot be opened is
    /// left to the listing, which names it unreadable.
    fn exists(&self, path: &Path) -> bool {
        let Some(name) = path.file_name() else {
            return true;
        };
        let parent = path.parent().unwrap_or(Path::new(""));
        match retained_directory(self.root, parent)
            .and_then(|directory| directory.retain_immediate_child_nofollow(name))
        {
            Ok(_) => true,
            Err(error) => error.kind() != io::ErrorKind::NotFound,
        }
    }

    fn entries(&self, path: &Path) -> io::Result<Vec<(OsString, TypedChildEntryKind)>> {
        self.check()?;
        let directory = retained_directory(self.root, path)?;
        let names = directory.read_immediate_names_with_limit(None, || self.check())?;
        names
            .into_iter()
            .map(|name| {
                let kind = match directory.retain_immediate_child_nofollow(&name)? {
                    SourceEntry::Directory(_) => TypedChildEntryKind::Directory,
                    SourceEntry::RegularFile(_) => TypedChildEntryKind::File,
                    SourceEntry::ReparsePoint => TypedChildEntryKind::Link,
                    SourceEntry::Unsupported => TypedChildEntryKind::Other,
                };
                Ok((name, kind))
            })
            .collect()
    }
}
