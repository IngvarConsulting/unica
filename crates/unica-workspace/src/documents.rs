//! Open editor buffers of a workspace, held by its actor as an overlay.
//!
//! A buffer belongs to one editor session and names one source file of one
//! source set. Analysis on behalf of that session reads the buffer instead of
//! the file; the file itself is never written here, and other sessions and
//! agent calls keep reading the disk.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The text of one buffer and the version its editor gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentText {
    pub version: i32,
    pub text: Arc<str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct DocumentKey {
    session: String,
    source_set: String,
    relative: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentOverlayError {
    /// The registry lock was poisoned by a panic.
    Unavailable,
    /// The version does not move forward.
    StaleVersion { current: i32, offered: i32 },
    /// No buffer of this session names the file.
    NotOpen,
}

/// Every open buffer of one workspace.
#[derive(Debug, Default)]
pub struct DocumentOverlay {
    buffers: Mutex<HashMap<DocumentKey, DocumentText>>,
}

/// The buffers of one session in one source set, by relative path: what a
/// read for that session overlays on the disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionDocuments {
    buffers: BTreeMap<PathBuf, DocumentText>,
}

impl SessionDocuments {
    pub fn get(&self, relative: &Path) -> Option<&DocumentText> {
        self.buffers.get(relative)
    }

    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }
}

impl DocumentOverlay {
    fn key(session: &str, source_set: &str, relative: &Path) -> DocumentKey {
        DocumentKey {
            session: session.to_owned(),
            source_set: source_set.to_owned(),
            relative: relative.to_owned(),
        }
    }

    /// Open a buffer, or replace an open one with a later version.
    pub fn put(
        &self,
        session: &str,
        source_set: &str,
        relative: &Path,
        document: DocumentText,
    ) -> Result<(), DocumentOverlayError> {
        let mut buffers = self
            .buffers
            .lock()
            .map_err(|_| DocumentOverlayError::Unavailable)?;
        let key = Self::key(session, source_set, relative);
        if let Some(current) = buffers.get(&key) {
            if document.version <= current.version {
                return Err(DocumentOverlayError::StaleVersion {
                    current: current.version,
                    offered: document.version,
                });
            }
        }
        buffers.insert(key, document);
        Ok(())
    }

    pub fn get(
        &self,
        session: &str,
        source_set: &str,
        relative: &Path,
    ) -> Result<Option<DocumentText>, DocumentOverlayError> {
        let buffers = self
            .buffers
            .lock()
            .map_err(|_| DocumentOverlayError::Unavailable)?;
        Ok(buffers
            .get(&Self::key(session, source_set, relative))
            .cloned())
    }

    pub fn remove(
        &self,
        session: &str,
        source_set: &str,
        relative: &Path,
    ) -> Result<DocumentText, DocumentOverlayError> {
        self.buffers
            .lock()
            .map_err(|_| DocumentOverlayError::Unavailable)?
            .remove(&Self::key(session, source_set, relative))
            .ok_or(DocumentOverlayError::NotOpen)
    }

    /// Drop every buffer of a session whose editor went away.
    pub fn close_session(&self, session: &str) -> Result<usize, DocumentOverlayError> {
        let mut buffers = self
            .buffers
            .lock()
            .map_err(|_| DocumentOverlayError::Unavailable)?;
        let before = buffers.len();
        buffers.retain(|key, _| key.session != session);
        Ok(before - buffers.len())
    }

    pub fn session_documents(
        &self,
        session: &str,
        source_set: &str,
    ) -> Result<SessionDocuments, DocumentOverlayError> {
        let buffers = self
            .buffers
            .lock()
            .map_err(|_| DocumentOverlayError::Unavailable)?;
        Ok(SessionDocuments {
            buffers: buffers
                .iter()
                .filter(|(key, _)| key.session == session && key.source_set == source_set)
                .map(|(key, document)| (key.relative.clone(), document.clone()))
                .collect(),
        })
    }

    /// The open files of every session in `source_set`, for telling editors
    /// that a published change touched them.
    pub fn open_in(
        &self,
        source_set: &str,
    ) -> Result<Vec<(String, PathBuf)>, DocumentOverlayError> {
        let buffers = self
            .buffers
            .lock()
            .map_err(|_| DocumentOverlayError::Unavailable)?;
        let mut open = buffers
            .keys()
            .filter(|key| key.source_set == source_set)
            .map(|key| (key.session.clone(), key.relative.clone()))
            .collect::<Vec<_>>();
        open.sort();
        Ok(open)
    }

    /// Whether any buffer is open; an actor holding one is not released.
    pub fn is_empty(&self) -> bool {
        self.buffers
            .lock()
            .map(|buffers| buffers.is_empty())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(version: i32, text: &str) -> DocumentText {
        DocumentText {
            version,
            text: Arc::from(text),
        }
    }

    #[test]
    fn buffers_are_kept_per_session_and_source_set_and_versions_only_grow() {
        let overlay = DocumentOverlay::default();
        let module = Path::new("CommonModules/Общий/Ext/Module.bsl");
        overlay.put("a", "main", module, text(1, "первый")).unwrap();
        overlay.put("b", "main", module, text(1, "второй")).unwrap();
        overlay
            .put("a", "ext", module, text(7, "расширение"))
            .unwrap();
        assert_eq!(
            overlay.put("a", "main", module, text(1, "назад")),
            Err(DocumentOverlayError::StaleVersion {
                current: 1,
                offered: 1
            })
        );
        overlay
            .put("a", "main", module, text(2, "первый, правка"))
            .unwrap();

        let a_main = overlay.session_documents("a", "main").unwrap();
        assert_eq!(a_main.get(module), Some(&text(2, "первый, правка")));
        assert_eq!(
            overlay.session_documents("b", "main").unwrap().get(module),
            Some(&text(1, "второй"))
        );
        assert!(overlay.session_documents("c", "main").unwrap().is_empty());
        assert_eq!(
            overlay.open_in("main").unwrap(),
            vec![
                ("a".to_owned(), module.to_owned()),
                ("b".to_owned(), module.to_owned())
            ]
        );

        assert_eq!(
            overlay.remove("b", "main", module).unwrap(),
            text(1, "второй")
        );
        assert_eq!(
            overlay.remove("b", "main", module),
            Err(DocumentOverlayError::NotOpen)
        );
        assert!(!overlay.is_empty());
        assert_eq!(overlay.close_session("a").unwrap(), 2);
        assert!(overlay.is_empty());
    }
}
