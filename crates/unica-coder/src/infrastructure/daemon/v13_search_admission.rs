//! Synchronous admission of a `unica.search` call (#1216).
//!
//! Search execution may wait for a long time: a provider role builds its
//! index, the names corpus builds its directory, a scoped text search reads
//! descriptors. A caller error found only after that wait arrives late, and
//! past the response cutoff it arrives as a task result. Everything that the
//! arguments alone decide is therefore checked here, before execution begins
//! and before any provider is asked whether it is ready: argument shape,
//! the page limit range, the scope grammar, and whether a cursor is an
//! authentic continuation of this very question — owner, query, scope, mode,
//! kind, source sets and page limit. Whether the answer behind a valid cursor
//! changed (`stale_cursor`) needs the answer and stays with execution.
//!
//! Execution keeps its own checks of the same arguments. The question binding
//! built here must equal the one execution builds, or a valid cursor would be
//! refused; the continuation tests of every search mode guard that equality.

use super::server::ActorBoundInvocation;
use crate::application::result_store::{SearchCursorBinding, SearchCursorStore};
use crate::application::v13::find::FindRequest;
use crate::domain::address::QualifiedAddress;
use crate::domain::code_intelligence::ProviderRole;
use crate::domain::invocation::DomainResult;
use crate::domain::refusal::RefusalCode;
use serde_json::{json, Map, Value};

const MAX_SEARCH_LIMIT: usize = 50;
const DEFAULT_SEARCH_LIMIT: usize = 20;

/// Refuses a search call that its arguments already decide, without reading
/// sources or touching a provider.
pub(super) fn admit_search_call(
    invocation: &ActorBoundInvocation,
    cursors: &SearchCursorStore,
) -> Result<(), Box<DomainResult>> {
    let arguments = invocation.arguments();
    let question = search_question(
        arguments,
        invocation.workspace_identity_hash().as_str(),
        &invocation.admitted_source_set_names(),
    )?;
    let cursor = match arguments.get("cursor") {
        None => return Ok(()),
        Some(Value::String(token)) => token,
        Some(_) => return Err(refusal("search cursor must be a string")),
    };
    // No question binding means the source selection itself is refused by
    // execution (an unadmitted scope, no admitted source set): that refusal
    // names the real cause, a cursor refusal would hide it.
    let Some(question) = question else {
        return Ok(());
    };
    cursors
        .verify_question(cursor, &question)
        .map_err(|error| Box::new(error.into_result(None)))
}

/// The question half of the cursor binding that execution will build for
/// these arguments, or a refusal of the arguments themselves.
fn search_question(
    arguments: &Map<String, Value>,
    workspace_identity: &str,
    admitted: &[&str],
) -> Result<Option<SearchCursorBinding>, Box<DomainResult>> {
    let Some(query) = arguments.get("query").and_then(Value::as_str) else {
        return Err(refusal("search requires string argument `query`"));
    };
    if query.trim().is_empty() {
        return Err(refusal("search query must not be blank"));
    }
    let corpus = match arguments.get("corpus") {
        None => "text",
        Some(Value::String(corpus)) => corpus.as_str(),
        Some(_) => return Err(refusal("search corpus must be a string")),
    };
    let question = |scope: Option<String>,
                    mode: &str,
                    kind: Option<String>,
                    source_sets: Vec<String>,
                    page_limit: usize| {
        (!source_sets.is_empty()).then(|| SearchCursorBinding {
            workspace_identity: workspace_identity.to_owned(),
            query: query.to_owned(),
            scope,
            mode: mode.to_owned(),
            kind,
            source_sets,
            result_fingerprint: None,
            page_limit,
        })
    };
    let selected = |scope: Option<&QualifiedAddress>| {
        admitted
            .iter()
            .filter(|name| scope.is_none_or(|scope| **name == scope.source_set()))
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    };
    match corpus {
        "names" => {
            if arguments.contains_key("role") {
                return Err(refusal("name search does not accept a provider role"));
            }
            let request = FindRequest::new(query)
                .map_err(|error| rejection(None, error.code(), error.to_string()))?;
            let kind = match arguments.get("kind") {
                None => None,
                Some(Value::String(kind)) => {
                    request
                        .with_kind(kind)
                        .map_err(|error| rejection(None, error.code(), error.to_string()))?;
                    Some(kind.clone())
                }
                Some(_) => return Err(refusal("search kind must be a string")),
            };
            let limit = page_limit(
                arguments,
                "search limit must be an integer from 1 through 50",
            )?;
            let scope = scope(arguments)?;
            Ok(question(
                scope.as_ref().map(ToString::to_string),
                "names",
                kind,
                selected(scope.as_ref()),
                limit,
            ))
        }
        "text" => match arguments.get("role") {
            None => {
                let mode = match arguments.get("regex") {
                    None | Some(Value::Bool(false)) => "literal",
                    Some(Value::Bool(true)) => {
                        // The same bounded compile execution performs.
                        regex::RegexBuilder::new(query)
                            .size_limit(1 << 20)
                            .dfa_size_limit(1 << 20)
                            .build()
                            .map_err(|error| {
                                refusal(format!("search regex is not a valid pattern: {error}"))
                            })?;
                        "regex"
                    }
                    Some(_) => return Err(refusal("search regex must be a boolean")),
                };
                let limit = page_limit(
                    arguments,
                    "search limit must be an integer from 1 through 50",
                )?;
                let scope = scope(arguments)?;
                Ok(question(
                    scope.as_ref().map(ToString::to_string),
                    mode,
                    None,
                    selected(scope.as_ref()),
                    limit,
                ))
            }
            Some(Value::String(role)) => {
                let Some(role) = ProviderRole::ALL
                    .into_iter()
                    .find(|candidate| candidate.as_str() == role)
                else {
                    let mut refused = refusal(format!(
                        "search role `{role}` is unknown; use one of {}",
                        ProviderRole::ALL
                            .iter()
                            .map(|role| format!("`{}`", role.as_str()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    refused.next.push(json!({
                        "tool": "unica.search",
                        "args": {"query": query},
                        "reason": "буквальный поиск без роли",
                    }));
                    return Err(refused);
                };
                let scope = scope(arguments)?;
                let limit = page_limit(
                    arguments,
                    "provider search page limit must be a positive integer at most 50",
                )?;
                // A role search binds the raw scope text, and searches either
                // every admitted set (unscoped lexical) or exactly one: the
                // scope's set, or the first admitted set without a scope.
                let source_sets = match scope.as_ref() {
                    Some(scope) => vec![scope.source_set().to_owned()],
                    None if role == ProviderRole::Lexical => selected(None),
                    None => admitted
                        .first()
                        .map(|name| vec![(*name).to_owned()])
                        .unwrap_or_default(),
                };
                Ok(question(
                    arguments
                        .get("scope")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    role.as_str(),
                    None,
                    source_sets,
                    limit,
                ))
            }
            Some(_) => Err(refusal("search role must be a string")),
        },
        other => {
            let mut refused = refusal(format!(
                "search corpus `{other}` is unknown; use `text` or `names`"
            ));
            refused.next.push(json!({
                "tool": "unica.search",
                "args": {"query": query, "corpus": "names"},
                "reason": "поиск по именам и синонимам метаданных",
            }));
            Err(refused)
        }
    }
}

fn page_limit(arguments: &Map<String, Value>, message: &str) -> Result<usize, Box<DomainResult>> {
    match arguments.get("limit") {
        None => Ok(DEFAULT_SEARCH_LIMIT),
        Some(value) => value
            .as_u64()
            .and_then(|limit| usize::try_from(limit).ok())
            .filter(|limit| (1..=MAX_SEARCH_LIMIT).contains(limit))
            .ok_or_else(|| refusal(message)),
    }
}

fn scope(arguments: &Map<String, Value>) -> Result<Option<QualifiedAddress>, Box<DomainResult>> {
    match arguments.get("scope") {
        None => Ok(None),
        Some(Value::String(scope)) => QualifiedAddress::parse(scope).map(Some).map_err(|error| {
            rejection(
                Some(scope.clone()),
                RefusalCode::BadValue,
                error.to_string(),
            )
        }),
        Some(_) => Err(refusal("search scope must be a string")),
    }
}

fn refusal(message: impl Into<String>) -> Box<DomainResult> {
    rejection(None, RefusalCode::BadValue, message)
}

fn rejection(
    at: Option<String>,
    code: RefusalCode,
    message: impl Into<String>,
) -> Box<DomainResult> {
    Box::new(DomainResult::canonical_rejection(at, code, message))
}
