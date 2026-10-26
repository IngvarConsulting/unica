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
use unica_bsl_parser::symbols::{LineIndex, LocalSymbol, LocalSymbolKind, NamePath};
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

/// Why an answer may miss something.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "detail")]
pub enum Incompleteness {
    /// The buffer differs from the saved module; the part answered beyond the
    /// module's own text describes the saved revision.
    Stale,
    /// Nobody could answer beyond the module's own text.
    ProviderUnavailable(String),
    /// A place the provider named has no logical address in this source set.
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

    fn partial(value: T, reasons: Vec<Incompleteness>) -> Self {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPlaces {
    pub places: Vec<ProviderPlace>,
    /// `false` when the provider capped or could not finish its answer.
    pub complete: bool,
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
    ) -> Result<Option<ProviderSymbol>, String>;

    fn references(
        &self,
        query: &SymbolQuery,
        context: &SymbolRequestContext<'_>,
    ) -> Result<ProviderPlaces, String>;

    /// The methods that call (`Incoming`) or are called by (`Outgoing`) the
    /// method the query names, each as the place of the other method's name.
    fn calls(
        &self,
        query: &SymbolQuery,
        direction: CallDirection,
        context: &SymbolRequestContext<'_>,
    ) -> Result<ProviderPlaces, String>;
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
}

/// The module a request is made in.
pub struct SymbolDocument<'a> {
    /// Logical address of the module.
    pub at: &'a str,
    /// Source-root-relative path of the saved module.
    pub path: &'a str,
    /// The text being edited.
    pub text: &'a str,
    /// Whether `text` is the saved module.
    pub saved: bool,
}

pub struct SymbolService<'a> {
    provider: Option<&'a dyn SymbolProvider>,
    environment: &'a dyn SymbolEnvironment,
}

/// What a position names, as far as the module's own text tells.
enum Resolved {
    Local(LocalSymbol),
    Metadata(SymbolLocation, String),
    /// A method of a common module: its location and name.
    CommonMethod(SymbolLocation, String),
    Unknown(NamePath),
}

impl<'a> SymbolService<'a> {
    pub fn new(
        provider: Option<&'a dyn SymbolProvider>,
        environment: &'a dyn SymbolEnvironment,
    ) -> Self {
        Self {
            provider,
            environment,
        }
    }

    fn resolve(
        &self,
        module: &unica_bsl_parser::ParsedModule<'_>,
        offset: usize,
    ) -> Option<Resolved> {
        let path = module.name_path_at(offset)?;
        let (head, _) = &path.segments[0];
        if path.cursor == 0 || path.segments.len() == 1 {
            let is_call = path.is_call && path.segments.len() == 1;
            if let Some(local) = module.local_definition(head, offset, is_call) {
                if path.cursor == 0 {
                    return Some(Resolved::Local(local));
                }
            }
        }
        if let Some(kind) = unica_address::source_target::metadata_collection_kind(head) {
            // `Справочники.Валюты` names the catalog; a segment past the
            // object is a manager method, which only a provider knows.
            if path.cursor == 1 {
                let object = &path.segments[1].0;
                let at = self.environment.metadata_address(kind, object, None)?;
                return Some(Resolved::Metadata(
                    SymbolLocation { at, range: None },
                    object.clone(),
                ));
            }
            if path.cursor == 0 {
                return None;
            }
        }
        if path.segments.len() >= 2 && path.cursor == 1 {
            if let Some(at) = self
                .environment
                .metadata_address("CommonModule", head, None)
            {
                let name = &path.segments[1].0;
                if let Some(location) = self.common_method(&at, name) {
                    return Some(Resolved::CommonMethod(location, name.clone()));
                }
            }
        }
        if path.cursor == 0 && path.segments.len() > 1 {
            if let Some(at) = self
                .environment
                .metadata_address("CommonModule", head, None)
            {
                return Some(Resolved::Metadata(
                    SymbolLocation { at, range: None },
                    head.clone(),
                ));
            }
        }
        Some(Resolved::Unknown(path))
    }

    fn common_method(&self, module_at: &str, name: &str) -> Option<SymbolLocation> {
        let text = self.environment.module_text(module_at)?;
        let module = unica_bsl_parser::parse(&text);
        let method = module
            .local_methods()
            .into_iter()
            .find(|method| method.name.to_lowercase() == name.to_lowercase())?;
        let index = LineIndex::new(&text);
        Some(SymbolLocation {
            at: module_at.to_string(),
            range: Some(range(&index, method.name_range)),
        })
    }

    fn query(
        &self,
        document: &SymbolDocument<'_>,
        path: &NamePath,
        position: SymbolPosition,
    ) -> SymbolQuery {
        if document.saved {
            SymbolQuery::At {
                path: document.path.to_string(),
                position,
            }
        } else {
            SymbolQuery::Named {
                name: path.segments[..=path.cursor]
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
            }
        }
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
        document: &SymbolDocument<'_>,
        ask: impl FnOnce(&dyn SymbolProvider) -> Result<T, String>,
    ) -> (Option<T>, Vec<Incompleteness>) {
        let Some(provider) = self.provider else {
            return (
                None,
                vec![Incompleteness::ProviderUnavailable(
                    "no symbol provider is configured".to_string(),
                )],
            );
        };
        let stale = if document.saved {
            Vec::new()
        } else {
            vec![Incompleteness::Stale]
        };
        match ask(provider) {
            Ok(value) => (Some(value), stale),
            Err(error) => (None, vec![Incompleteness::ProviderUnavailable(error)]),
        }
    }

    fn local_location(
        &self,
        document: &SymbolDocument<'_>,
        index: &LineIndex<'_>,
        range_bytes: std::ops::Range<usize>,
    ) -> SymbolLocation {
        SymbolLocation {
            at: document.at.to_string(),
            range: Some(range(index, range_bytes)),
        }
    }

    /// Where the name under `position` is declared.
    pub fn definition(
        &self,
        document: &SymbolDocument<'_>,
        position: SymbolPosition,
        context: &SymbolRequestContext<'_>,
    ) -> SymbolAnswer<Vec<SymbolLocation>> {
        let index = LineIndex::new(document.text);
        let offset = index.offset(position.line, position.column);
        let module = unica_bsl_parser::parse(document.text);
        match self.resolve(&module, offset) {
            None => SymbolAnswer::complete(Vec::new()),
            Some(Resolved::Local(local)) => {
                SymbolAnswer::complete(vec![self.local_location(document, &index, local.range)])
            }
            Some(Resolved::Metadata(location, _)) | Some(Resolved::CommonMethod(location, _)) => {
                SymbolAnswer::complete(vec![location])
            }
            Some(Resolved::Unknown(path)) => {
                let query = self.query(document, &path, position);
                let (symbol, mut reasons) =
                    self.ask(document, |provider| provider.symbol(&query, context));
                let locations = symbol
                    .flatten()
                    .map(|symbol| self.places(&symbol.definitions, &mut reasons))
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
        let index = LineIndex::new(document.text);
        let offset = index.offset(position.line, position.column);
        let module = unica_bsl_parser::parse(document.text);
        match self.resolve(&module, offset) {
            None => SymbolAnswer::complete(None),
            Some(Resolved::Local(local)) => {
                let kind = match local.kind {
                    LocalSymbolKind::Method => SymbolKind::Method,
                    LocalSymbolKind::Parameter => SymbolKind::Parameter,
                    LocalSymbolKind::Variable | LocalSymbolKind::ModuleVariable => {
                        SymbolKind::Variable
                    }
                };
                let location = self.local_location(document, &index, local.range.clone());
                SymbolAnswer::complete(Some(SymbolInfo {
                    name: local.name,
                    kind,
                    signature: None,
                    return_type: None,
                    location: Some(location),
                }))
            }
            Some(Resolved::Metadata(location, name)) => SymbolAnswer::complete(Some(SymbolInfo {
                name,
                kind: SymbolKind::Metadata,
                signature: None,
                return_type: None,
                location: Some(location),
            })),
            Some(Resolved::CommonMethod(location, name)) => {
                SymbolAnswer::complete(Some(SymbolInfo {
                    name,
                    kind: SymbolKind::Method,
                    signature: None,
                    return_type: None,
                    location: Some(location),
                }))
            }
            Some(Resolved::Unknown(path)) => {
                let query = self.query(document, &path, position);
                let (symbol, mut reasons) =
                    self.ask(document, |provider| provider.symbol(&query, context));
                let info = symbol.flatten().map(|symbol| {
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
    /// included. A parameter or variable is answered from the module alone; a
    /// method of the module also by the provider, for its callers elsewhere.
    pub fn references(
        &self,
        document: &SymbolDocument<'_>,
        position: SymbolPosition,
        context: &SymbolRequestContext<'_>,
    ) -> SymbolAnswer<Vec<SymbolLocation>> {
        let index = LineIndex::new(document.text);
        let offset = index.offset(position.line, position.column);
        let module = unica_bsl_parser::parse(document.text);
        let resolved = self.resolve(&module, offset);
        let mut locations = Vec::new();
        let mut reasons = Vec::new();
        let Some(resolved) = resolved else {
            return SymbolAnswer::complete(locations);
        };
        if let Resolved::Local(local) = &resolved {
            locations.extend(
                module
                    .local_references(local)
                    .into_iter()
                    .map(|range| self.local_location(document, &index, range)),
            );
            if local.kind != LocalSymbolKind::Method {
                return SymbolAnswer::complete(locations);
            }
        }
        let Some(path) = module.name_path_at(offset) else {
            return SymbolAnswer::complete(locations);
        };
        let query = self.query(document, &path, position);
        let (answer, more) = self.ask(document, |provider| provider.references(&query, context));
        reasons.extend(more);
        if let Some(answer) = answer {
            if !answer.complete {
                reasons.push(Incompleteness::ProviderUnavailable(
                    "the provider capped its references".to_string(),
                ));
            }
            for location in self.places(&answer.places, &mut reasons) {
                // The module's own places come from its text, which may be a
                // buffer the provider has not seen.
                if location.at != document.at && !locations.contains(&location) {
                    locations.push(location);
                }
            }
        }
        SymbolAnswer::partial(locations, reasons)
    }

    /// The methods calling, or called by, the method under `position`.
    pub fn calls(
        &self,
        document: &SymbolDocument<'_>,
        position: SymbolPosition,
        direction: CallDirection,
        context: &SymbolRequestContext<'_>,
    ) -> SymbolAnswer<Vec<SymbolLocation>> {
        let index = LineIndex::new(document.text);
        let offset = index.offset(position.line, position.column);
        let module = unica_bsl_parser::parse(document.text);
        let methods = module.local_methods();
        let mut locations: Vec<SymbolLocation> = Vec::new();
        let push = |location: SymbolLocation, locations: &mut Vec<SymbolLocation>| {
            if !locations.contains(&location) {
                locations.push(location);
            }
        };
        let Some(Resolved::Local(local)) = self.resolve(&module, offset) else {
            return self.provider_calls(
                document,
                &module,
                offset,
                position,
                direction,
                context,
                Vec::new(),
            );
        };
        if local.kind != LocalSymbolKind::Method {
            return SymbolAnswer::complete(Vec::new());
        }
        let Some(method) = methods
            .iter()
            .find(|method| method.name_range == local.range)
        else {
            return SymbolAnswer::complete(Vec::new());
        };
        match direction {
            CallDirection::Outgoing => {
                for call in module.calls_in(method) {
                    if let [name] = call.path.as_slice() {
                        if let Some(callee) = methods
                            .iter()
                            .find(|candidate| candidate.name.to_lowercase() == name.to_lowercase())
                        {
                            push(
                                self.local_location(document, &index, callee.name_range.clone()),
                                &mut locations,
                            );
                        }
                    } else if let [module_name, name] = call.path.as_slice() {
                        if let Some(at) =
                            self.environment
                                .metadata_address("CommonModule", module_name, None)
                        {
                            if let Some(location) = self.common_method(&at, name) {
                                push(location, &mut locations);
                            }
                        }
                    }
                }
            }
            CallDirection::Incoming => {
                // One pass over the module's calls; each call belongs to the
                // method whose declaration holds it.
                let name = method.name.to_lowercase();
                for call in module.calls() {
                    if !matches!(call.path.as_slice(), [called] if called.to_lowercase() == name) {
                        continue;
                    }
                    if let Some(caller) = methods
                        .iter()
                        .find(|caller| caller.range.contains(&call.range.start))
                    {
                        push(
                            self.local_location(document, &index, caller.name_range.clone()),
                            &mut locations,
                        );
                    }
                }
            }
        }
        self.provider_calls(
            document, &module, offset, position, direction, context, locations,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn provider_calls(
        &self,
        document: &SymbolDocument<'_>,
        module: &unica_bsl_parser::ParsedModule<'_>,
        offset: usize,
        position: SymbolPosition,
        direction: CallDirection,
        context: &SymbolRequestContext<'_>,
        mut locations: Vec<SymbolLocation>,
    ) -> SymbolAnswer<Vec<SymbolLocation>> {
        let Some(path) = module.name_path_at(offset) else {
            return SymbolAnswer::complete(locations);
        };
        let query = self.query(document, &path, position);
        let (answer, mut reasons) = self.ask(document, |provider| {
            provider.calls(&query, direction, context)
        });
        if let Some(answer) = answer {
            if !answer.complete {
                reasons.push(Incompleteness::ProviderUnavailable(
                    "the provider capped its call hierarchy".to_string(),
                ));
            }
            for location in self.places(&answer.places, &mut reasons) {
                if location.at != document.at && !locations.contains(&location) {
                    locations.push(location);
                }
            }
        }
        SymbolAnswer::partial(locations, reasons)
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
