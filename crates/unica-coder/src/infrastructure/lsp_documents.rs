//! Editor documents of one LSP connection (#2174).
//!
//! A document is the body of a BSL module named by
//! `unica://<space>/<set>/<address>`, where the address is the one MCP gives
//! that text: `CommonModule.X.Body`, `Catalog.X.Module.Object.Body`. The
//! connection keeps the buffers its editor opened and mirrors each version
//! into the overlay of the workspace actor, so analysis on behalf of this
//! connection reads the buffer; the file on disk is written only by
//! `unica.apply`. A file path (`file://`) never names a document.

use crate::application::lsp::document::{ContentChange, DocumentChangeError, OpenDocument};
use crate::application::lsp::uri::{DocumentUri, SpaceRegistry, UriError, WorkspaceFolder};
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use unica_address::address::QualifiedAddress;
use unica_bsl::profile::PlatformProfile;
use unica_execution_contract::view_error::ViewError;
use unica_source_contract::SourceSetKind;
use unica_source_designer::module_layout::{module_relative, module_source_address};
use unica_workspace::documents::{DocumentOverlay, DocumentOverlayError, DocumentText};

/// One admitted source set of a space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceSetLayout {
    pub(crate) name: String,
    pub(crate) kind: SourceSetKind,
    pub(crate) root: PathBuf,
    pub(crate) profile: PlatformProfile,
}

/// A workspace folder the editor opened and the source sets admitted in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpaceLayout {
    pub(crate) folder: WorkspaceFolder,
    pub(crate) source_sets: Vec<SourceSetLayout>,
}

#[derive(Debug)]
pub(crate) enum LspDocumentError {
    Uri(UriError),
    /// The address is not the `Body` of a BSL module of the source set
    /// layout.
    NotAModule,
    UnknownSourceSet(String),
    NotOpen,
    Change(DocumentChangeError),
    Overlay(DocumentOverlayError),
    Read(ViewError),
}

/// The documents of one LSP connection.
#[derive(Debug)]
pub(crate) struct LspDocuments {
    /// Keys this connection's buffers in every actor overlay.
    session: String,
    spaces: SpaceRegistry,
    source_sets: BTreeMap<String, Vec<SourceSetLayout>>,
    open: BTreeMap<DocumentUri, OpenBuffer>,
}

#[derive(Debug, Clone)]
struct OpenBuffer {
    location: DocumentLocation,
    document: OpenDocument,
}

/// Where a document lives: its space, source set and module file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentLocation {
    pub(crate) document: DocumentUri,
    pub(crate) source_set: String,
    pub(crate) relative: PathBuf,
}

impl LspDocuments {
    pub(crate) fn new(session: impl Into<String>, mut layouts: Vec<SpaceLayout>) -> Self {
        // Roots are compared with the root of a read, which may name them
        // through a symlink (`/tmp` and `/private/tmp` on macOS).
        for layout in &mut layouts {
            for set in &mut layout.source_sets {
                set.root = canonical(&set.root);
            }
        }
        let spaces = SpaceRegistry::from_folders(
            &layouts
                .iter()
                .map(|layout| layout.folder.clone())
                .collect::<Vec<_>>(),
        );
        let mut source_sets = BTreeMap::new();
        for layout in layouts {
            if let Some(space) = spaces.space_of_root(&layout.folder.root) {
                source_sets
                    .entry(space.to_owned())
                    .or_insert(layout.source_sets);
            }
        }
        Self {
            session: session.into(),
            spaces,
            source_sets,
            open: BTreeMap::new(),
        }
    }

    pub(crate) fn session(&self) -> &str {
        &self.session
    }

    pub(crate) fn spaces(&self) -> &SpaceRegistry {
        &self.spaces
    }

    /// The space, source set and module file a `unica://` URI names.
    pub(crate) fn locate(&self, uri: &str) -> Result<DocumentLocation, LspDocumentError> {
        let document = self.spaces.parse(uri).map_err(LspDocumentError::Uri)?;
        let at = module_of_body(&document)?;
        let layout = self.source_set(&document.space, at.source_set())?;
        let relative = module_file(&at, layout).ok_or(LspDocumentError::NotAModule)?;
        Ok(DocumentLocation {
            source_set: layout.name.clone(),
            relative,
            document,
        })
    }

    /// `didOpen`: the buffer replaces the file for this connection.
    pub(crate) fn open(
        &mut self,
        overlay: &DocumentOverlay,
        uri: &str,
        version: i32,
        text: String,
    ) -> Result<DocumentLocation, LspDocumentError> {
        let location = self.locate(uri)?;
        // A reopen starts a new buffer life; its versions start over.
        if self.open.remove(&location.document).is_some() {
            overlay
                .remove(&self.session, &location.source_set, &location.relative)
                .map_err(LspDocumentError::Overlay)?;
        }
        let document = OpenDocument::new(version, text);
        self.mirror(overlay, &location, &document)?;
        self.open.insert(
            location.document.clone(),
            OpenBuffer {
                location: location.clone(),
                document,
            },
        );
        Ok(location)
    }

    /// `didChange`: applies the change set atomically, then mirrors it.
    pub(crate) fn change(
        &mut self,
        overlay: &DocumentOverlay,
        uri: &str,
        version: i32,
        changes: &[ContentChange],
    ) -> Result<DocumentLocation, LspDocumentError> {
        let location = self.locate(uri)?;
        let mut document = self
            .open
            .get(&location.document)
            .map(|buffer| buffer.document.clone())
            .ok_or(LspDocumentError::NotOpen)?;
        document
            .change(version, changes)
            .map_err(LspDocumentError::Change)?;
        self.mirror(overlay, &location, &document)?;
        if let Some(buffer) = self.open.get_mut(&location.document) {
            buffer.document = document;
        }
        Ok(location)
    }

    /// `didClose`: the connection reads the file again.
    pub(crate) fn close(
        &mut self,
        overlay: &DocumentOverlay,
        uri: &str,
    ) -> Result<DocumentLocation, LspDocumentError> {
        let location = self.locate(uri)?;
        self.open
            .remove(&location.document)
            .ok_or(LspDocumentError::NotOpen)?;
        overlay
            .remove(&self.session, &location.source_set, &location.relative)
            .map_err(LspDocumentError::Overlay)?;
        Ok(location)
    }

    pub(crate) fn open_document(&self, document: &DocumentUri) -> Option<&OpenDocument> {
        self.open.get(document).map(|buffer| &buffer.document)
    }

    /// `workspace/textDocumentContent`: the whole module, from the buffer
    /// when this connection has one open, otherwise from `read`. The text
    /// is not cut to a model budget; the editor asked for the file.
    pub(crate) fn content(
        &self,
        uri: &str,
        read: &ProviderReadAuthority,
    ) -> Result<Option<String>, LspDocumentError> {
        let location = self.locate(uri)?;
        if let Some(buffer) = self.open.get(&location.document) {
            return Ok(Some(buffer.document.text().to_owned()));
        }
        let layout = self.source_set(&location.document.space, &location.source_set)?;
        // Two spaces may both have a `main`: the read must be of this one.
        if read.source_set() != location.source_set || canonical(read.root_path()) != layout.root {
            return Err(LspDocumentError::UnknownSourceSet(location.source_set));
        }
        let at = module_of_body(&location.document)?;
        let target = module_target(&at, layout).ok_or(LspDocumentError::NotAModule)?;
        read.module_source(&target).map_err(LspDocumentError::Read)
    }

    /// `textDocument/signatureHelp` at `position` of `uri`. A qualified
    /// call names a common module of the same source set, read through
    /// `read` (with this connection's buffers).
    pub(crate) fn signature_help(
        &self,
        uri: &str,
        position: crate::application::lsp::position::Utf16Position,
        read: &ProviderReadAuthority,
    ) -> Result<Option<serde_json::Value>, LspDocumentError> {
        let Some(text) = self.content(uri, read)? else {
            return Ok(None);
        };
        let location = self.locate(uri)?;
        let common_module = |name: &str| {
            let document = DocumentUri {
                space: location.document.space.clone(),
                address: format!("{}:CommonModule.{name}", location.source_set),
            };
            let uri = self.spaces.format(&document).ok()?;
            self.content(&uri, read).ok().flatten()
        };
        Ok(crate::infrastructure::lsp_features::signature_help(
            &text,
            position,
            &common_module,
        ))
    }

    /// The `unica://` URIs to refresh after `apply` published `relatives`
    /// of a source set in a space: every module this connection opened
    /// there whose file changed. After an epoch change pass `None`: every
    /// open module of the space is stale.
    pub(crate) fn changed_documents(
        &self,
        space: &str,
        source_set: &str,
        relatives: Option<&[PathBuf]>,
    ) -> Vec<String> {
        self.open
            .values()
            .map(|buffer| &buffer.location)
            .filter(|location| {
                location.document.space == space
                    && location.source_set == source_set
                    && relatives.is_none_or(|relatives| relatives.contains(&location.relative))
            })
            .filter_map(|location| self.spaces.format(&location.document).ok())
            .collect()
    }

    fn source_set(&self, space: &str, name: &str) -> Result<&SourceSetLayout, LspDocumentError> {
        self.source_sets
            .get(space)
            .and_then(|sets| sets.iter().find(|set| set.name == name))
            .ok_or_else(|| LspDocumentError::UnknownSourceSet(name.to_owned()))
    }

    fn mirror(
        &self,
        overlay: &DocumentOverlay,
        location: &DocumentLocation,
        document: &OpenDocument,
    ) -> Result<(), LspDocumentError> {
        overlay
            .put(
                &self.session,
                &location.source_set,
                &location.relative,
                DocumentText {
                    version: document.version(),
                    text: Arc::from(document.text()),
                },
            )
            .map_err(LspDocumentError::Overlay)
    }
}

/// The path with symlinks resolved, or as given when it does not exist.
fn canonical(path: &std::path::Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

/// The module whose `Body` a document address names.
fn module_of_body(document: &DocumentUri) -> Result<QualifiedAddress, LspDocumentError> {
    let module = document
        .address
        .strip_suffix(".Body")
        .ok_or(LspDocumentError::NotAModule)?;
    QualifiedAddress::parse(module)
        .map_err(|error| LspDocumentError::Uri(UriError::Address(error.to_string())))
}

fn module_target(
    at: &QualifiedAddress,
    layout: &SourceSetLayout,
) -> Option<unica_address::source_target::MetadataAddress> {
    let capability = layout.profile.module_capability(at)?;
    module_source_address(
        at,
        capability.owner_kind(),
        capability.role(),
        capability.route_kind(),
    )
    .ok()
}

/// The module file of a logical module address, relative to its source set.
fn module_file(at: &QualifiedAddress, layout: &SourceSetLayout) -> Option<PathBuf> {
    module_relative(&module_target(at, layout)?, layout.kind).ok()
}

#[cfg(test)]
#[path = "../../tests/platform/lsp_documents.rs"]
mod tests;
