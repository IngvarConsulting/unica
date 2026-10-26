//! Symbols of BSL code in Unica's own model: where a name is declared, what it
//! is, where it is used and which methods call each other.
//!
//! The answer never names its provider. The module's own text answers for the
//! names it declares and for metadata it names (`Справочники.Валюты`); a
//! [`SymbolProvider`] answers for the rest. A provider reads saved files only,
//! so an edited buffer is answered from its own text, and anything a provider
//! still answers is marked [`Incompleteness::Stale`]: it describes the saved
//! revision, not the buffer.

use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashMap;
use unica_bsl_parser::symbols::{
    same_name, LineIndex, LocalMethod, LocalSymbol, LocalSymbolKind, NamePath,
};
use unica_bsl_parser::ParsedModule;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

/// A position in a module: 0-based line and 0-based UTF-16 column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolPosition {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolRange {
    pub start: SymbolPosition,
    pub end: SymbolPosition,
}

/// A place in Unica's terms: a logical address and, inside a module, a range.
/// A metadata node has no range: the place is the node itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolLocation {
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<SymbolRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SymbolKind {
    Method,
    Parameter,
    Variable,
    Metadata,
    Platform,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<SymbolLocation>,
}

/// Why an answer may miss something. Unica's own codes: a provider's words
/// never reach the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "count")]
pub enum Incompleteness {
    /// The buffer differs from the saved module; the part answered beyond the
    /// module's own text describes the saved revision.
    Stale,
    /// The provider is still preparing this answer; asking again later can
    /// give the full one.
    ProviderNotReady,
    /// No provider could answer beyond the module's own text.
    ProviderUnavailable,
    /// The provider answered with part of what it found, or from an index
    /// behind the saved sources.
    Capped,
    /// Places in other modules are not answered: only this module's are.
    OtherModules,
    /// The name index of the source set is still being built; asking again
    /// later can give the places in other modules too.
    NamesNotIndexed,
    /// Uses through a variable holding an object are not found: the code
    /// does not say what a variable holds.
    UntypedReferences,
    /// So many places the provider named have no logical address here.
    Unaddressable(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolAnswer<T> {
    pub value: T,
    pub complete: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<Incompleteness>,
}

impl<T> SymbolAnswer<T> {
    fn complete(value: T) -> Self {
        Self {
            value,
            complete: true,
            reasons: Vec::new(),
        }
    }

    fn partial(value: T, mut reasons: Vec<Incompleteness>) -> Self {
        reasons.dedup();
        Self {
            complete: reasons.is_empty(),
            value,
            reasons,
        }
    }
}

/// What a provider is asked about: a position in a saved module, or a name
/// when the buffer has moved positions away from the saved file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolQuery {
    At {
        /// Source-root-relative, `/`-separated path of the saved module.
        path: String,
        position: SymbolPosition,
    },
    Named {
        name: String,
        /// The module the name is written in: a bare name is looked up there
        /// first, so another module's method of the same name is not taken.
        path: String,
    },
}

/// A place in a provider's terms: a file range, or a metadata node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderPlace {
    Code {
        path: String,
        range: SymbolRange,
    },
    /// `kind` is the canonical English metadata kind (`Catalog`); `member` an
    /// attribute, dimension or resource of the object.
    Metadata {
        kind: String,
        object: String,
        member: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub signature: Option<String>,
    pub return_type: Option<String>,
    pub definitions: Vec<ProviderPlace>,
    /// `false` when the provider's index is behind the sources or its lookup
    /// was capped.
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPlaces {
    pub places: Vec<ProviderPlace>,
    /// `false` when the provider capped or could not finish its answer.
    pub complete: bool,
}

/// Why a provider gave no answer, in Unica's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFailure {
    /// Still indexing, or out of the time this request allowed.
    NotReady,
    Unavailable,
}

impl ProviderFailure {
    const fn reason(self) -> Incompleteness {
        match self {
            Self::NotReady => Incompleteness::ProviderNotReady,
            Self::Unavailable => Incompleteness::ProviderUnavailable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirection {
    Incoming,
    Outgoing,
}

pub struct SymbolRequestContext<'a> {
    pub deadline: ProviderDeadline,
    pub cancellation: &'a CancellationToken,
}

/// The replaceable source of symbols beyond a module's own text. It answers in
/// file paths and metadata names; the service turns them into Unica places.
pub trait SymbolProvider: Send + Sync {
    fn symbol(
        &self,
        query: &SymbolQuery,
        context: &SymbolRequestContext<'_>,
    ) -> Result<Option<ProviderSymbol>, ProviderFailure>;

    /// Every place that names the symbol, by a walk over the configuration.
    /// The editor's references do not call it: the walk is the slow first
    /// pass the background index of #2190 is to run instead, and the shipped
    /// analyzer session does not enable its `references` tool until then.
    fn references(
        &self,
        query: &SymbolQuery,
        context: &SymbolRequestContext<'_>,
    ) -> Result<ProviderPlaces, ProviderFailure>;

    /// The methods that call (`Incoming`) or are called by (`Outgoing`) the
    /// method the query names, each as the place of the other method's name.
    fn calls(
        &self,
        query: &SymbolQuery,
        direction: CallDirection,
        context: &SymbolRequestContext<'_>,
    ) -> Result<ProviderPlaces, ProviderFailure>;
}

/// What the service needs from the source set: module addresses and texts,
/// and proof that a metadata node exists.
pub trait SymbolEnvironment {
    /// The module address of a source-root-relative file path.
    fn module_address(&self, path: &str) -> Option<String>;
    /// The address of a metadata node the source set declares: the object
    /// `kind.object`, or its field `member` of whatever field kind it is.
    fn metadata_address(&self, kind: &str, object: &str, member: Option<&str>) -> Option<String>;
    /// The saved text of the module at a logical address, for a common
    /// module named in code.
    fn module_text(&self, at: &str) -> Option<String>;
    /// What the module at a logical address belongs to, for the way other
    /// modules name its exports.
    fn module_owner(&self, at: &str) -> ModuleOwner;
    /// The saved text of the module at a source-root-relative path.
    fn module_source(&self, path: &str) -> Option<String>;
}

/// What a module belongs to, as far as other modules name its exports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleOwner {
    /// A common module: its exports are `Name.Method`, or bare when global.
    Common { name: String, global: bool },
    /// The manager module of an object: `Справочники.Name.Method`.
    Manager { kind: String, object: String },
    /// Any other module: its exports are reached through objects.
    Other,
}

/// Which modules of the source set may spell a name: the name index.
pub trait NameLookup {
    /// The source-root-relative paths of the modules that spell `name` in any
    /// letter case, or `None` while the index is not built yet.
    fn modules_naming(&self, name: &str) -> Option<Vec<String>>;
}

/// `Справочники.Валюты.Метод` with the cursor on the method: an export of
/// the catalog's manager module.
fn manager_method(path: &NamePath) -> Option<Target> {
    if !path.plain || path.cursor != 2 || path.segments.len() < 3 {
        return None;
    }
    let kind = unica_address::source_target::metadata_collection_kind(&path.segments[0].0)?;
    Some(Target::Manager {
        kind: kind.to_string(),
        object: path.segments[1].0.clone(),
        method: path.segments[2].0.clone(),
    })
}

/// What other modules call a symbol by, for finding its uses there.
enum Target {
    /// `module.method`, or a bare `method` when the module is global.
    Common {
        module: String,
        method: String,
        global: bool,
    },
    /// `Справочники.object.method` and the like.
    Manager {
        kind: String,
        object: String,
        method: String,
    },
    /// `Справочники.object`, or a common module's own name.
    Metadata { kind: String, object: String },
}

impl Target {
    /// The name every use spells: the last segment.
    fn name(&self) -> &str {
        match self {
            Self::Common { method, .. } | Self::Manager { method, .. } => method,
            Self::Metadata { object, .. } => object,
        }
    }

    /// The range of this target's name in each path that names it.
    fn uses(&self, module: &ParsedModule<'_>) -> Vec<std::ops::Range<usize>> {
        let collection = |raw: &str, kind: &str| {
            unica_address::source_target::metadata_collection_kind(raw) == Some(kind)
        };
        module
            .paths_naming(&unica_bsl_parser::symbols::name_key(self.name()))
            .into_iter()
            .filter(|path| path.plain || path.cursor == 0)
            .filter(|path| {
                let segment = |at: usize| path.segments.get(at).map(|(name, _)| name.as_str());
                match self {
                    Self::Common {
                        module: name,
                        global,
                        ..
                    } => match path.cursor {
                        1 => segment(0).is_some_and(|head| same_name(head, name)),
                        0 => {
                            *global
                                && path.is_call
                                && module
                                    .local_definition(
                                        &path.segments[0].0,
                                        path.segments[0].1.start,
                                        true,
                                    )
                                    .is_none()
                        }
                        _ => false,
                    },
                    Self::Manager { kind, object, .. } => {
                        path.cursor == 2
                            && segment(0).is_some_and(|head| collection(head, kind))
                            && segment(1).is_some_and(|name| same_name(name, object))
                    }
                    Self::Metadata { kind, .. } if kind == "CommonModule" => {
                        path.cursor == 0
                            && path.plain
                            && path.segments.len() >= 2
                            && module
                                .local_definition(
                                    &path.segments[0].0,
                                    path.segments[0].1.start,
                                    false,
                                )
                                .is_none()
                    }
                    Self::Metadata { kind, .. } => {
                        path.cursor == 1 && segment(0).is_some_and(|head| collection(head, kind))
                    }
                }
            })
            .map(|path| path.segments[path.cursor].1.clone())
            .collect()
    }
}

/// The module a request is made in.
pub struct SymbolDocument<'a> {
    /// Logical address of the module.
    pub at: &'a str,
    /// Source-root-relative path of the saved module.
    pub path: &'a str,
    /// The text being edited.
    pub text: &'a str,
    /// The saved text of the module.
    pub saved: &'a str,
}

impl SymbolDocument<'_> {
    fn is_saved(&self) -> bool {
        self.text.trim_start_matches('\u{feff}') == self.saved.trim_start_matches('\u{feff}')
    }
}

/// The answer for an edited buffer whose name the provider cannot be asked
/// about: what the buffer alone tells, marked stale.
fn unasked<T>(value: T) -> SymbolAnswer<T> {
    SymbolAnswer::partial(value, vec![Incompleteness::Stale])
}

/// What a position names, as far as the module's own text tells.
enum Resolved {
    Local(LocalSymbol),
    /// A metadata object: its location, name and kind.
    Metadata(SymbolLocation, String, &'static str),
    /// An exported method of a common module: its location and name.
    CommonMethod(SymbolLocation, String),
    Unknown(NamePath),
}

/// A common module named in code, parsed once per request.
struct CommonModule {
    text: String,
    methods: Vec<LocalMethod>,
}

pub struct SymbolService<'a> {
    provider: Option<&'a dyn SymbolProvider>,
    environment: &'a dyn SymbolEnvironment,
    names: Option<&'a dyn NameLookup>,
    common_modules: RefCell<HashMap<String, Option<CommonModule>>>,
}

/// One request's parsed module and its line index.
struct Request<'d, 't> {
    document: &'d SymbolDocument<'t>,
    module: ParsedModule<'t>,
    index: LineIndex<'t>,
    offset: usize,
    position: SymbolPosition,
}

impl<'d, 't> Request<'d, 't> {
    fn new(document: &'d SymbolDocument<'t>, position: SymbolPosition) -> Self {
        let index = LineIndex::new(document.text);
        let offset = index.offset(position.line, position.column);
        Self {
            document,
            module: unica_bsl_parser::parse(document.text),
            index,
            offset,
            position,
        }
    }

    fn location(&self, bytes: std::ops::Range<usize>) -> SymbolLocation {
        SymbolLocation {
            at: self.document.at.to_string(),
            range: Some(range(&self.index, bytes)),
        }
    }
}

impl<'a> SymbolService<'a> {
    pub fn new(
        provider: Option<&'a dyn SymbolProvider>,
        environment: &'a dyn SymbolEnvironment,
    ) -> Self {
        Self {
            provider,
            environment,
            names: None,
            common_modules: RefCell::new(HashMap::new()),
        }
    }

    /// Answers references in other modules from the source set's name index.
    pub fn with_names(mut self, names: &'a dyn NameLookup) -> Self {
        self.names = Some(names);
        self
    }

    fn resolve(&self, request: &Request<'_, '_>) -> Option<Resolved> {
        let path = request.module.name_path_at(request.offset)?;
        if !path.plain {
            return Some(Resolved::Unknown(path));
        }
        let (head, _) = &path.segments[0];
        if path.cursor == 0 {
            let is_call = path.is_call && path.segments.len() == 1;
            if let Some(local) = request
                .module
                .local_definition(head, request.offset, is_call)
            {
                return Some(Resolved::Local(local));
            }
        }
        if let Some(kind) = unica_address::source_target::metadata_collection_kind(head) {
            // `Справочники.Валюты` names the catalog; a segment past the
            // object is a manager method, which only a provider knows.
            if path.cursor == 0 {
                return None;
            }
            if path.cursor == 1 {
                let object = &path.segments[1].0;
                if let Some(at) = self.environment.metadata_address(kind, object, None) {
                    return Some(Resolved::Metadata(
                        SymbolLocation { at, range: None },
                        object.clone(),
                        kind,
                    ));
                }
            }
            return Some(Resolved::Unknown(path));
        }
        if path.segments.len() >= 2 && path.cursor <= 1 {
            if let Some(at) = self
                .environment
                .metadata_address("CommonModule", head, None)
            {
                if path.cursor == 0 {
                    return Some(Resolved::Metadata(
                        SymbolLocation { at, range: None },
                        head.clone(),
                        "CommonModule",
                    ));
                }
                let name = &path.segments[1].0;
                if let Some(location) = self.common_method(&at, name) {
                    return Some(Resolved::CommonMethod(location, name.clone()));
                }
            }
        }
        Some(Resolved::Unknown(path))
    }

    /// An exported method of a common module, by the module's saved text.
    fn common_method(&self, module_at: &str, name: &str) -> Option<SymbolLocation> {
        let mut modules = self.common_modules.borrow_mut();
        let module = modules
            .entry(module_at.to_string())
            .or_insert_with(|| {
                let text = self.environment.module_text(module_at)?;
                let methods = unica_bsl_parser::parse(&text).local_methods();
                Some(CommonModule { text, methods })
            })
            .as_ref()?;
        let method = module
            .methods
            .iter()
            .find(|method| method.is_export && same_name(&method.name, name))?;
        Some(SymbolLocation {
            at: module_at.to_string(),
            range: Some(range(
                &LineIndex::new(&module.text),
                method.name_range.clone(),
            )),
        })
    }

    /// The provider query for the name under the position. A saved module is
    /// asked by position. In an edited buffer a name the module declares is
    /// asked at its declaration in the saved text, and any other by name.
    /// A name declared only in the buffer has no query: the provider has not
    /// seen it.
    fn query(&self, request: &Request<'_, '_>, resolved: &Resolved) -> Option<SymbolQuery> {
        let document = request.document;
        if document.is_saved() {
            return Some(SymbolQuery::At {
                path: document.path.to_string(),
                position: request.position,
            });
        }
        if let Resolved::Local(local) = resolved {
            let saved = unica_bsl_parser::parse(document.saved);
            let declared = match local.kind {
                LocalSymbolKind::Method => saved
                    .local_methods()
                    .into_iter()
                    .find(|method| same_name(&method.name, &local.name))
                    .map(|method| method.name_range),
                LocalSymbolKind::ModuleVariable => saved
                    .local_definition(&local.name, 0, false)
                    .filter(|symbol| symbol.kind == LocalSymbolKind::ModuleVariable)
                    .map(|symbol| symbol.range),
                _ => None,
            }?;
            let (line, column) = LineIndex::new(document.saved).position(declared.start);
            return Some(SymbolQuery::At {
                path: document.path.to_string(),
                position: SymbolPosition { line, column },
            });
        }
        let path = match resolved {
            Resolved::Unknown(path) => path.clone(),
            _ => request.module.name_path_at(request.offset)?,
        };
        Some(SymbolQuery::Named {
            name: path.segments[..=path.cursor]
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join("."),
            path: document.path.to_string(),
        })
    }

    fn place(&self, place: &ProviderPlace) -> Option<SymbolLocation> {
        match place {
            ProviderPlace::Code { path, range } => Some(SymbolLocation {
                at: self.environment.module_address(path)?,
                range: Some(*range),
            }),
            ProviderPlace::Metadata {
                kind,
                object,
                member,
            } => Some(SymbolLocation {
                at: self
                    .environment
                    .metadata_address(kind, object, member.as_deref())?,
                range: None,
            }),
        }
    }

    fn places(
        &self,
        places: &[ProviderPlace],
        reasons: &mut Vec<Incompleteness>,
    ) -> Vec<SymbolLocation> {
        let mut unaddressable = 0;
        let locations = places
            .iter()
            .filter_map(|place| {
                let location = self.place(place);
                unaddressable += usize::from(location.is_none());
                location
            })
            .collect();
        if unaddressable > 0 {
            reasons.push(Incompleteness::Unaddressable(unaddressable));
        }
        locations
    }

    fn ask<T>(
        &self,
        request: &Request<'_, '_>,
        ask: impl FnOnce(&dyn SymbolProvider) -> Result<T, ProviderFailure>,
    ) -> (Option<T>, Vec<Incompleteness>) {
        let Some(provider) = self.provider else {
            return (None, vec![Incompleteness::ProviderUnavailable]);
        };
        let stale = if request.document.is_saved() {
            Vec::new()
        } else {
            vec![Incompleteness::Stale]
        };
        match ask(provider) {
            Ok(value) => (Some(value), stale),
            Err(failure) => (None, vec![failure.reason()]),
        }
    }

    /// Provider places joined to `locations`. Where the module's own text has
    /// already answered for this module, the provider's places in it are not
    /// repeated: the buffer may have moved them.
    fn join(
        &self,
        answer: Option<ProviderPlaces>,
        reasons: &mut Vec<Incompleteness>,
        locations: &mut Vec<SymbolLocation>,
        own_module: Option<&str>,
    ) {
        let Some(answer) = answer else {
            return;
        };
        if !answer.complete {
            reasons.push(Incompleteness::Capped);
        }
        for location in self.places(&answer.places, reasons) {
            if Some(location.at.as_str()) != own_module && !locations.contains(&location) {
                locations.push(location);
            }
        }
    }

    /// Where the name under `position` is declared.
    pub fn definition(
        &self,
        document: &SymbolDocument<'_>,
        position: SymbolPosition,
        context: &SymbolRequestContext<'_>,
    ) -> SymbolAnswer<Vec<SymbolLocation>> {
        if context.cancellation.is_cancelled() {
            return SymbolAnswer::partial(Vec::new(), vec![Incompleteness::ProviderNotReady]);
        }
        let request = Request::new(document, position);
        match self.resolve(&request) {
            None => SymbolAnswer::complete(Vec::new()),
            Some(Resolved::Local(local)) => {
                SymbolAnswer::complete(vec![request.location(local.range)])
            }
            Some(Resolved::Metadata(location, ..)) | Some(Resolved::CommonMethod(location, _)) => {
                SymbolAnswer::complete(vec![location])
            }
            Some(unknown @ Resolved::Unknown(_)) => {
                let Some(query) = self.query(&request, &unknown) else {
                    return unasked(Vec::new());
                };
                let (symbol, mut reasons) =
                    self.ask(&request, |provider| provider.symbol(&query, context));
                let locations = symbol
                    .flatten()
                    .map(|symbol| {
                        if !symbol.complete {
                            reasons.push(Incompleteness::Capped);
                        }
                        self.places(&symbol.definitions, &mut reasons)
                    })
                    .unwrap_or_default();
                SymbolAnswer::partial(locations, reasons)
            }
        }
    }

    /// What the name under `position` is.
    pub fn hover(
        &self,
        document: &SymbolDocument<'_>,
        position: SymbolPosition,
        context: &SymbolRequestContext<'_>,
    ) -> SymbolAnswer<Option<SymbolInfo>> {
        if context.cancellation.is_cancelled() {
            return SymbolAnswer::partial(None, vec![Incompleteness::ProviderNotReady]);
        }
        let request = Request::new(document, position);
        let info = |name, kind, location| SymbolInfo {
            name,
            kind,
            signature: None,
            return_type: None,
            location: Some(location),
        };
        match self.resolve(&request) {
            None => SymbolAnswer::complete(None),
            Some(Resolved::Local(local)) => {
                let kind = match local.kind {
                    LocalSymbolKind::Method => SymbolKind::Method,
                    LocalSymbolKind::Parameter => SymbolKind::Parameter,
                    LocalSymbolKind::Variable | LocalSymbolKind::ModuleVariable => {
                        SymbolKind::Variable
                    }
                };
                let location = request.location(local.range.clone());
                SymbolAnswer::complete(Some(info(local.name, kind, location)))
            }
            Some(Resolved::Metadata(location, name, _)) => {
                SymbolAnswer::complete(Some(info(name, SymbolKind::Metadata, location)))
            }
            Some(Resolved::CommonMethod(location, name)) => {
                SymbolAnswer::complete(Some(info(name, SymbolKind::Method, location)))
            }
            Some(unknown @ Resolved::Unknown(_)) => {
                let Some(query) = self.query(&request, &unknown) else {
                    return unasked(None);
                };
                let (symbol, mut reasons) =
                    self.ask(&request, |provider| provider.symbol(&query, context));
                let info = symbol.flatten().map(|symbol| {
                    if !symbol.complete {
                        reasons.push(Incompleteness::Capped);
                    }
                    let location = self
                        .places(&symbol.definitions, &mut reasons)
                        .into_iter()
                        .next();
                    SymbolInfo {
                        name: symbol.name,
                        kind: symbol.kind,
                        signature: symbol.signature,
                        return_type: symbol.return_type,
                        location,
                    }
                });
                SymbolAnswer::partial(info, reasons)
            }
        }
    }

    /// Every place that names the symbol under `position`, its declaration
    /// included. A parameter or local variable is answered from the module
    /// alone. A symbol other modules name is looked for in the modules the
    /// name index says spell its name, each parsed and its paths resolved;
    /// the provider is never asked, since its walk reads the whole corpus.
    pub fn references(
        &self,
        document: &SymbolDocument<'_>,
        position: SymbolPosition,
        context: &SymbolRequestContext<'_>,
    ) -> SymbolAnswer<Vec<SymbolLocation>> {
        if context.cancellation.is_cancelled() {
            return SymbolAnswer::partial(Vec::new(), vec![Incompleteness::ProviderNotReady]);
        }
        let request = Request::new(document, position);
        let Some(resolved) = self.resolve(&request) else {
            return SymbolAnswer::complete(Vec::new());
        };
        let mut locations = Vec::new();
        let mut reasons = Vec::new();
        let target = match &resolved {
            Resolved::Local(local) => {
                locations.extend(
                    request
                        .module
                        .local_references(local)
                        .into_iter()
                        .map(|range| request.location(range)),
                );
                if matches!(
                    local.kind,
                    LocalSymbolKind::Parameter | LocalSymbolKind::Variable
                ) || !request.module.is_exported(local)
                {
                    return SymbolAnswer::complete(locations);
                }
                match (local.kind, self.environment.module_owner(document.at)) {
                    (LocalSymbolKind::Method, ModuleOwner::Common { name, global }) => {
                        Target::Common {
                            module: name,
                            method: local.name.clone(),
                            global,
                        }
                    }
                    (LocalSymbolKind::Method, ModuleOwner::Manager { kind, object }) => {
                        Target::Manager {
                            kind,
                            object,
                            method: local.name.clone(),
                        }
                    }
                    _ => {
                        return SymbolAnswer::partial(
                            locations,
                            vec![Incompleteness::UntypedReferences],
                        )
                    }
                }
            }
            Resolved::Metadata(_, object, kind) => Target::Metadata {
                kind: (*kind).to_string(),
                object: object.clone(),
            },
            Resolved::CommonMethod(location, method) => {
                match self.environment.module_owner(&location.at) {
                    ModuleOwner::Common { name, global } => Target::Common {
                        module: name,
                        method: method.clone(),
                        global,
                    },
                    _ => {
                        return SymbolAnswer::partial(
                            Vec::new(),
                            vec![Incompleteness::OtherModules],
                        )
                    }
                }
            }
            Resolved::Unknown(path) => match manager_method(path) {
                Some(target) => target,
                None => {
                    return SymbolAnswer::partial(Vec::new(), vec![Incompleteness::OtherModules])
                }
            },
        };
        for range in target.uses(&request.module) {
            let location = request.location(range);
            if !locations.contains(&location) {
                locations.push(location);
            }
        }
        let Some(names) = self.names else {
            return SymbolAnswer::partial(locations, vec![Incompleteness::OtherModules]);
        };
        let Some(paths) = names.modules_naming(target.name()) else {
            return SymbolAnswer::partial(locations, vec![Incompleteness::NamesNotIndexed]);
        };
        let mut unaddressable = 0;
        for path in paths {
            if context.cancellation.is_cancelled() {
                reasons.push(Incompleteness::ProviderNotReady);
                break;
            }
            let Some(at) = self.environment.module_address(&path) else {
                unaddressable += 1;
                continue;
            };
            if at == document.at {
                continue;
            }
            let Some(text) = self.environment.module_source(&path) else {
                unaddressable += 1;
                continue;
            };
            let module = unica_bsl_parser::parse(&text);
            let index = LineIndex::new(&text);
            locations.extend(
                target
                    .uses(&module)
                    .into_iter()
                    .map(|bytes| SymbolLocation {
                        at: at.clone(),
                        range: Some(range(&index, bytes)),
                    }),
            );
        }
        if unaddressable > 0 {
            reasons.push(Incompleteness::Unaddressable(unaddressable));
        }
        if matches!(target, Target::Manager { .. }) {
            // The manager's exports are also reached through a variable
            // holding the manager, which the code does not type.
            reasons.push(Incompleteness::UntypedReferences);
        }
        if reasons.is_empty() {
            SymbolAnswer::complete(locations)
        } else {
            SymbolAnswer::partial(locations, reasons)
        }
    }

    /// The methods calling, or called by, the method under `position`.
    pub fn calls(
        &self,
        document: &SymbolDocument<'_>,
        position: SymbolPosition,
        direction: CallDirection,
        context: &SymbolRequestContext<'_>,
    ) -> SymbolAnswer<Vec<SymbolLocation>> {
        if context.cancellation.is_cancelled() {
            return SymbolAnswer::partial(Vec::new(), vec![Incompleteness::ProviderNotReady]);
        }
        let request = Request::new(document, position);
        let Some(resolved) = self.resolve(&request) else {
            return SymbolAnswer::complete(Vec::new());
        };
        let mut locations: Vec<SymbolLocation> = Vec::new();
        let mut own_module = None;
        if let Resolved::Local(local) = &resolved {
            if local.kind != LocalSymbolKind::Method {
                return SymbolAnswer::complete(Vec::new());
            }
            let methods = request.module.local_methods();
            let Some(method) = methods
                .iter()
                .find(|method| method.name_range == local.range)
            else {
                return SymbolAnswer::complete(Vec::new());
            };
            self.local_calls(&request, &methods, method, direction, &mut locations);
            if context.cancellation.is_cancelled() {
                return SymbolAnswer::partial(locations, vec![Incompleteness::ProviderNotReady]);
            }
            own_module = Some(document.at);
        } else if !matches!(resolved, Resolved::Unknown(_) | Resolved::CommonMethod(..)) {
            return SymbolAnswer::complete(Vec::new());
        }
        let Some(query) = self.query(&request, &resolved) else {
            return unasked(locations);
        };
        let (answer, mut reasons) = self.ask(&request, |provider| {
            provider.calls(&query, direction, context)
        });
        self.join(answer, &mut reasons, &mut locations, own_module);
        SymbolAnswer::partial(locations, reasons)
    }

    /// The calls between `method` and the module's own methods, and the
    /// exported common-module methods it calls, by the buffer.
    fn local_calls(
        &self,
        request: &Request<'_, '_>,
        methods: &[LocalMethod],
        method: &LocalMethod,
        direction: CallDirection,
        locations: &mut Vec<SymbolLocation>,
    ) {
        let mut push = |location: SymbolLocation| {
            if !locations.contains(&location) {
                locations.push(location);
            }
        };
        let calls = request.module.calls();
        match direction {
            CallDirection::Outgoing => {
                for call in calls
                    .iter()
                    .filter(|call| method.range.contains(&call.range.start))
                {
                    match call.path.as_slice() {
                        [name] => {
                            if let Some(callee) = methods
                                .iter()
                                .find(|candidate| same_name(&candidate.name, name))
                            {
                                push(request.location(callee.name_range.clone()));
                            }
                        }
                        [module, name] => {
                            if let Some(at) =
                                self.environment
                                    .metadata_address("CommonModule", module, None)
                            {
                                if let Some(location) = self.common_method(&at, name) {
                                    push(location);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            CallDirection::Incoming => {
                // One pass over the module's calls; each call belongs to the
                // method whose declaration holds it.
                for call in &calls {
                    if !matches!(call.path.as_slice(), [called] if same_name(called, &method.name))
                    {
                        continue;
                    }
                    if let Some(caller) = methods
                        .iter()
                        .find(|caller| caller.range.contains(&call.range.start))
                    {
                        push(request.location(caller.name_range.clone()));
                    }
                }
            }
        }
    }
}

fn range(index: &LineIndex<'_>, bytes: std::ops::Range<usize>) -> SymbolRange {
    let (start_line, start_column) = index.position(bytes.start);
    let (end_line, end_column) = index.position(bytes.end);
    SymbolRange {
        start: SymbolPosition {
            line: start_line,
            column: start_column,
        },
        end: SymbolPosition {
            line: end_line,
            column: end_column,
        },
    }
}

#[cfg(test)]
mod tests;
