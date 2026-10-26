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
use crate::infrastructure::lsp_features::{CompletionSources, ModuleOwner, ObjectMembers};
use crate::infrastructure::lsp_types::TypeSources;
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use unica_address::address::QualifiedAddress;
use unica_bsl::profile::PlatformProfile;
use unica_bsl_types::{AppliedMember, AppliedMemberKind, AppliedTypes, PlatformCatalog};
use unica_documentation_sources::platform_help::provider::GlobalContextMember;
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
    /// The answer needs a source that is not ready yet (the platform
    /// help); answering without it would pass a partial answer as whole.
    SourceNotReady,
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

    /// The document URI of a module file, for tests that start from a
    /// fixture path: the module address `unica.resolve` gives it, plus `Body`.
    #[cfg(test)]
    pub(crate) fn body_uri_of_file(&self, path: &std::path::Path) -> String {
        let path = canonical(path);
        let (space, set, relative) = self
            .source_sets
            .iter()
            .flat_map(|(space, sets)| sets.iter().map(move |set| (space, set)))
            .find_map(|(space, set)| Some((space, set, path.strip_prefix(&set.root).ok()?)))
            .expect("the file is in a source set");
        let at = crate::infrastructure::event_sources::module_address_for_relative(
            &set.name,
            set.kind,
            set.profile,
            relative,
        )
        .expect("the file is a module");
        self.spaces
            .format(&DocumentUri {
                space: space.clone(),
                address: format!("{at}.Body"),
            })
            .expect("the space is this connection's")
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
    /// `read` (with this connection's buffers); a plain call not declared
    /// by the module is looked up in `global_context`, which is `None`
    /// while the platform help is still being indexed.
    pub(crate) fn signature_help(
        &self,
        uri: &str,
        position: crate::application::lsp::position::Utf16Position,
        read: &ProviderReadAuthority,
        global_context: Option<&[GlobalContextMember]>,
        catalog: Option<&PlatformCatalog>,
    ) -> Result<Option<serde_json::Value>, LspDocumentError> {
        let Some(text) = self.content(uri, read)? else {
            return Ok(None);
        };
        let location = self.locate(uri)?;
        let common_module = |name: &str| self.common_module_content(&location, name, read);
        let sources = self.sources(
            &location,
            read,
            global_context.unwrap_or_default(),
            catalog,
            &|| false,
        );
        crate::infrastructure::lsp_features::signature_help(
            &text,
            position,
            &common_module,
            global_context,
            sources.types().as_ref(),
        )
        .map_err(|_| LspDocumentError::SourceNotReady)
    }

    /// `textDocument/hover`: the type of the expression under the cursor
    /// or the signature of the method it calls, from the platform catalog.
    /// `None` when the catalog is not loaded or the type is not known.
    pub(crate) fn hover(
        &self,
        uri: &str,
        position: crate::application::lsp::position::Utf16Position,
        read: &ProviderReadAuthority,
        catalog: Option<&PlatformCatalog>,
    ) -> Result<Option<serde_json::Value>, LspDocumentError> {
        let Some(text) = self.content(uri, read)? else {
            return Ok(None);
        };
        let location = self.locate(uri)?;
        let sources = self.sources(&location, read, &[], catalog, &|| false);
        Ok(sources
            .types()
            .and_then(|types| crate::infrastructure::lsp_types::hover(&text, position, &types)))
    }

    fn sources<'a>(
        &'a self,
        location: &'a DocumentLocation,
        read: &'a ProviderReadAuthority,
        global_context: &'a [GlobalContextMember],
        catalog: Option<&'a PlatformCatalog>,
        stale: &'a dyn Fn() -> bool,
    ) -> CompletionSourcesOver<'a> {
        CompletionSourcesOver {
            applied: MetadataGraphAppliedTypes {
                documents: self,
                location,
                read,
            },
            global_context,
            catalog,
            stale,
        }
    }

    /// The text of the common module `typed` names in the source set of
    /// `location`: its buffer, otherwise its file.
    fn common_module_content(
        &self,
        location: &DocumentLocation,
        typed: &str,
        read: &ProviderReadAuthority,
    ) -> Option<String> {
        let name = self.common_module_name(location, typed);
        let document = DocumentUri {
            space: location.document.space.clone(),
            address: format!("{}:CommonModule.{name}.Body", location.source_set),
        };
        let uri = self.spaces.format(&document).ok()?;
        self.content(&uri, read).ok().flatten()
    }

    /// `textDocument/completion` at `position` of `uri`. The answer is
    /// dropped (`None`) when the buffer moved past the version it was
    /// computed for, or when `cancelled` says the request is gone.
    // The request, the read and the platform's two sources (the help's
    // global context and the type catalog) come from the route as is.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn completion(
        &self,
        overlay: &DocumentOverlay,
        uri: &str,
        position: crate::application::lsp::position::Utf16Position,
        read: &ProviderReadAuthority,
        global_context: &[GlobalContextMember],
        catalog: Option<&PlatformCatalog>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<serde_json::Value>, LspDocumentError> {
        let Some(text) = self.content(uri, read)? else {
            return Ok(None);
        };
        let location = self.locate(uri)?;
        let version = self
            .open
            .get(&location.document)
            .map(|buffer| buffer.document.version());
        let sources = CompletionSourcesOver {
            applied: MetadataGraphAppliedTypes {
                documents: self,
                location: &location,
                read,
            },
            global_context,
            catalog,
            stale: &|| {
                cancelled()
                    || version.is_some_and(|version| {
                        // A newer `didChange` reached the overlay; a failed
                        // read of it says nothing about the edit.
                        overlay
                            .get(&self.session, &location.source_set, &location.relative)
                            .ok()
                            .flatten()
                            .is_some_and(|buffer| buffer.version != version)
                    })
            },
        };
        Ok(crate::infrastructure::lsp_features::completion(
            &text, position, &sources,
        ))
    }

    /// `completionItem/resolve` for an item `completion` gave on `uri`.
    pub(crate) fn resolve_completion(
        &self,
        uri: &str,
        item: serde_json::Value,
        read: &ProviderReadAuthority,
    ) -> Result<serde_json::Value, LspDocumentError> {
        let location = self.locate(uri)?;
        let text = match item["data"]["commonModule"].as_str() {
            Some(module) => self.common_module_content(&location, module, read),
            None => self.content(uri, read)?,
        };
        Ok(match text {
            Some(text) => crate::infrastructure::lsp_features::resolve_completion(item, &text),
            None => item,
        })
    }

    /// The common module `typed` names, as its buffer or its folder spells
    /// it: names compare case-insensitively, buffers and files do not.
    fn common_module_name(&self, location: &DocumentLocation, typed: &str) -> String {
        let wanted = typed.to_lowercase();
        let prefix = format!("{}:CommonModule.", location.source_set);
        let open = self.open.keys().find_map(|document| {
            let name = document
                .address
                .strip_prefix(&prefix)?
                .strip_suffix(".Body")?;
            (document.space == location.document.space && name.to_lowercase() == wanted)
                .then(|| name.to_owned())
        });
        open.or_else(|| {
            let layout = self
                .source_set(&location.document.space, &location.source_set)
                .ok()?;
            std::fs::read_dir(layout.root.join("CommonModules"))
                .ok()?
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .find(|name| name.to_lowercase() == wanted)
        })
        .unwrap_or_else(|| typed.to_owned())
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

/// The applied half of the type model read from the source set's metadata
/// graph and module texts. A stand-in behind [`AppliedTypes`]: the applied
/// model of stage 2 (#2238) replaces this one type, and nothing that infers
/// types changes.
struct MetadataGraphAppliedTypes<'a> {
    documents: &'a LspDocuments,
    location: &'a DocumentLocation,
    read: &'a ProviderReadAuthority,
}

impl MetadataGraphAppliedTypes<'_> {
    fn registered(&self, kind: &str) -> Vec<String> {
        self.read
            .registered_names(kind, &|| Ok(()))
            .unwrap_or_default()
    }

    fn object_members(&self, kind: &str, name: &str) -> Option<ObjectMembers> {
        let target = unica_address::source_target::MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{kind}.{name}"),
        )
        .ok()?;
        let info = self.read.metadata_local(&target, &mut || Ok(())).ok()?.info;
        let collections = &info.collections;
        Some(ObjectMembers {
            attributes: collections
                .attributes
                .iter()
                .map(|attribute| attribute.name.clone())
                .collect(),
            tabular_sections: collections
                .tabular_sections
                .iter()
                .map(|section| {
                    (
                        section.name.clone(),
                        section
                            .attributes
                            .iter()
                            .map(|column| column.name.clone())
                            .collect(),
                    )
                })
                .collect(),
        })
    }

    /// Exported methods of the `role` module (`Manager`, `Object`) of
    /// `kind` `name`, from its buffer or file.
    fn module_exports(&self, kind: &str, name: &str, role: &str) -> Vec<AppliedMember> {
        let document = DocumentUri {
            space: self.location.document.space.clone(),
            address: format!(
                "{}:{kind}.{name}.Module.{role}.Body",
                self.location.source_set
            ),
        };
        let Some(text) = self
            .documents
            .spaces
            .format(&document)
            .ok()
            .and_then(|uri| self.documents.content(&uri, self.read).ok().flatten())
        else {
            return Vec::new();
        };
        crate::infrastructure::lsp_features::exported_methods(&text)
            .into_iter()
            .map(|(method, params)| AppliedMember {
                name: method,
                kind: AppliedMemberKind::Method,
                types: Vec::new(),
                signature: Some(unica_bsl_types::Signature {
                    name: String::new(),
                    params: params
                        .into_iter()
                        .map(|param| unica_bsl_types::Param {
                            ru: param,
                            en: String::new(),
                            optional: false,
                            types: Vec::new(),
                            default: None,
                        })
                        .collect(),
                    returns: Vec::new(),
                }),
            })
            .collect()
    }
}

impl AppliedTypes for MetadataGraphAppliedTypes<'_> {
    fn names(&self, kind: &str) -> Vec<String> {
        self.registered(kind)
    }

    fn members(&self, kind: &str, name: &str, facet: &str) -> Vec<AppliedMember> {
        let mut members = Vec::new();
        if matches!(facet, "Object" | "Ref") {
            if let Some(found) = self.object_members(kind, name) {
                members.extend(found.attributes.into_iter().map(|attribute| AppliedMember {
                    name: attribute,
                    kind: AppliedMemberKind::Attribute,
                    types: Vec::new(),
                    signature: None,
                }));
                members.extend(found.tabular_sections.into_iter().map(|(section, _)| {
                    AppliedMember {
                        name: section,
                        kind: AppliedMemberKind::TabularSection,
                        types: Vec::new(),
                        signature: None,
                    }
                }));
            }
        }
        match facet {
            "Object" => members.extend(self.module_exports(kind, name, "Object")),
            "Manager" => members.extend(self.module_exports(kind, name, "Manager")),
            _ => {}
        }
        members
    }
}

/// Completion sources of one document: the metadata of its source set,
/// read through the connection's read.
struct CompletionSourcesOver<'a> {
    applied: MetadataGraphAppliedTypes<'a>,
    global_context: &'a [GlobalContextMember],
    catalog: Option<&'a PlatformCatalog>,
    stale: &'a dyn Fn() -> bool,
}

impl CompletionSources for CompletionSourcesOver<'_> {
    fn global_context(&self) -> &[GlobalContextMember] {
        self.global_context
    }

    fn metadata_names(&self, kind: &str) -> Vec<String> {
        self.applied.registered(kind)
    }

    fn common_module_text(&self, name: &str) -> Option<String> {
        self.applied
            .documents
            .common_module_content(self.applied.location, name, self.applied.read)
    }

    fn object_members(&self, kind: &str, name: &str) -> Option<ObjectMembers> {
        self.applied.object_members(kind, name)
    }

    fn module_owner(&self) -> Option<ModuleOwner> {
        // The object module and the forms of an object reach its members.
        // The document address is the module's `Body`.
        let address = &self.applied.location.document.address;
        let address = address.strip_suffix(".Body").unwrap_or(address);
        let form = address.ends_with(".Module.Form") && address.contains(".Form.");
        if !(address.ends_with(".Module.Object") || form) {
            return None;
        }
        let at = QualifiedAddress::parse(address).ok()?;
        let owner = at.segments().first()?;
        Some(ModuleOwner {
            kind: owner.kind().as_str().to_owned(),
            name: owner.name()?.to_owned(),
            form,
        })
    }

    fn cancelled(&self) -> bool {
        (self.stale)()
    }

    fn types(&self) -> Option<TypeSources<'_>> {
        Some(TypeSources {
            catalog: self.catalog?,
            applied: &self.applied,
            owner: self.module_owner(),
        })
    }
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
