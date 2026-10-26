//! The analyzer behind Unica's symbol port. Its `symbol_info`, `references`
//! and call `graph` answer in file ranges and graph ids; this adapter turns
//! them into provider places and Unica's failure codes, and never lets the
//! analyzer's own names or words through.

use serde_json::{json, Value};
use unica_code_intelligence::model::CodeIntelligenceContext;
use unica_code_intelligence::symbols::{
    CallDirection, ProviderFailure, ProviderPlace, ProviderPlaces, ProviderSymbol, SymbolKind,
    SymbolPosition, SymbolProvider, SymbolQuery, SymbolRange, SymbolRequestContext,
};

use crate::provider::BslToolClient;

/// Neighbours asked for one call-hierarchy answer; a larger answer is capped
/// and reported incomplete.
const CALL_NEIGHBOURS: usize = 500;

pub struct BslAnalyzerSymbols<'a> {
    client: &'a (dyn BslToolClient + Send + Sync),
    context: CodeIntelligenceContext,
}

impl<'a> BslAnalyzerSymbols<'a> {
    pub fn new(
        client: &'a (dyn BslToolClient + Send + Sync),
        context: CodeIntelligenceContext,
    ) -> Self {
        Self { client, context }
    }

    /// One tool call. Running out of the request's time is "not ready". The
    /// analysis service still retires the shared analyzer session on such a
    /// call, so a caller passes the request's own deadline, never a shorter
    /// editor budget, until #2189 cancels without retiring it.
    fn call(
        &self,
        tool: &'static str,
        arguments: Value,
        context: &SymbolRequestContext<'_>,
    ) -> Result<Value, ProviderFailure> {
        if context.cancellation.is_cancelled() {
            return Err(ProviderFailure::NotReady);
        }
        let timeout = context
            .deadline
            .finite_remaining()
            .map_err(|_| ProviderFailure::NotReady)?;
        let output = self
            .client
            .call(
                tool,
                &self.context,
                arguments,
                timeout,
                context.cancellation,
            )
            .map_err(|_| {
                if context.deadline.is_elapsed() || context.cancellation.is_cancelled() {
                    ProviderFailure::NotReady
                } else {
                    ProviderFailure::Unavailable
                }
            })?;
        let answer: Value = serde_json::from_str(output.result_text.trim())
            .map_err(|_| ProviderFailure::Unavailable)?;
        if answer.get("status").and_then(Value::as_str) == Some("loading") {
            return Err(ProviderFailure::NotReady);
        }
        Ok(answer)
    }

    /// The analyzer's `symbol_info` for a query: the symbol at a position, or
    /// the exact candidate of a name, and whether its answer was fresh.
    fn symbol_info(
        &self,
        query: &SymbolQuery,
        context: &SymbolRequestContext<'_>,
    ) -> Result<Option<(Value, bool)>, ProviderFailure> {
        let answer = self.call("symbol_info", anchor(query), context)?;
        let fresh = is_fresh(&answer);
        if answer.get("status").and_then(Value::as_str) == Some("ok") {
            return Ok(Some((answer, fresh)));
        }
        match query {
            SymbolQuery::At { .. } => Ok(None),
            SymbolQuery::Named { name, path } => Ok(named_candidate(&answer, name, path)
                .cloned()
                .map(|candidate| (candidate, fresh))),
        }
    }
}

/// Whether an answer reports its index current and its lookup whole.
fn is_fresh(answer: &Value) -> bool {
    let freshness = &answer["freshness"];
    freshness["stale"] == false && freshness["completeness"]["status"] == "complete"
}

/// The candidate a name stands for. A bare name is the method of the module
/// it is written in when that module has it; otherwise it must be the only
/// exact candidate, or nothing is claimed.
fn named_candidate<'v>(answer: &'v Value, name: &str, path: &str) -> Option<&'v Value> {
    let exact: Vec<&Value> = answer["candidates"]
        .as_array()?
        .iter()
        .filter(|candidate| {
            candidate["match"] == "exact" && candidate["address"]["location"].is_object()
        })
        .collect();
    if !name.contains('.') {
        if let Some(own) = exact
            .iter()
            .find(|candidate| candidate["address"]["location"]["path"] == path)
        {
            return Some(own);
        }
    }
    match exact.as_slice() {
        [only] => Some(only),
        _ => None,
    }
}

fn anchor(query: &SymbolQuery) -> Value {
    match query {
        SymbolQuery::At { path, position } => {
            json!({"path": path, "line": position.line, "column": position.column})
        }
        SymbolQuery::Named { name, .. } => json!({"symbol": name}),
    }
}

fn position(value: &Value, line: &str, column: &str) -> Option<SymbolPosition> {
    Some(SymbolPosition {
        line: usize::try_from(value.get(line)?.as_u64()?).ok()?,
        column: usize::try_from(value.get(column)?.as_u64()?).ok()?,
    })
}

/// A file place from an analyzer location: its declared-name range.
fn code_place(location: &Value) -> Option<ProviderPlace> {
    if location.get("position_encoding").and_then(Value::as_str) != Some("utf-16") {
        return None;
    }
    let range = location.get("range")?;
    Some(ProviderPlace::Code {
        path: location.get("path")?.as_str()?.to_string(),
        range: SymbolRange {
            start: position(range, "start_line", "start_character")?,
            end: position(range, "end_line", "end_character")?,
        },
    })
}

/// A metadata place from a graph id: an object or its field.
fn metadata_place(graph_id: &str) -> Option<ProviderPlace> {
    let (prefix, rest) = graph_id.split_once('/')?;
    let parts: Vec<&str> = rest.split('/').collect();
    match (prefix, parts.as_slice()) {
        ("mdo", [kind, object]) => Some(ProviderPlace::Metadata {
            kind: (*kind).to_string(),
            object: (*object).to_string(),
            member: None,
        }),
        ("attribute", [kind, object, member]) => Some(ProviderPlace::Metadata {
            kind: (*kind).to_string(),
            object: (*object).to_string(),
            member: Some((*member).to_string()),
        }),
        _ => None,
    }
}

fn kind(kind: &str) -> SymbolKind {
    let kind = kind.to_lowercase();
    if kind.contains("metadata") {
        SymbolKind::Metadata
    } else if kind.contains("method") || kind == "procedure" || kind == "function" {
        SymbolKind::Method
    } else if kind.contains("parameter") {
        SymbolKind::Parameter
    } else if kind.contains("variable") {
        SymbolKind::Variable
    } else {
        SymbolKind::Platform
    }
}

fn graph_id(info: &Value) -> Option<&str> {
    info.get("graph_id")
        .or_else(|| info.get("usages").and_then(|usages| usages.get("graph_id")))
        .or_else(|| {
            info.get("address")
                .and_then(|address| address.get("graph_id"))
        })
        .and_then(Value::as_str)
}

/// A `symbol_info` answer, or an exact name candidate, as a provider symbol;
/// `fresh` tells whether the answer it came from was.
pub fn parse_symbol(info: &Value, fresh: bool) -> Option<ProviderSymbol> {
    let candidate = info.get("address").is_some();
    let mut definitions: Vec<ProviderPlace> = if candidate {
        code_place(&info["address"]["location"])
            .into_iter()
            .collect()
    } else {
        info["definitions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|definition| code_place(&definition["location"]))
            .collect()
    };
    if let Some(place) = graph_id(info).and_then(metadata_place) {
        definitions = vec![place];
    }
    let (name, kind_name) = if candidate {
        (
            info.get("display")?.as_str()?,
            info.get("category")?.as_str()?,
        )
    } else {
        (info.get("symbol")?.as_str()?, info.get("kind")?.as_str()?)
    };
    Some(ProviderSymbol {
        name: name.to_string(),
        kind: kind(kind_name),
        signature: info
            .get("signature")
            .and_then(Value::as_str)
            .map(str::to_string),
        return_type: info
            .get("return_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        definitions,
        complete: fresh,
    })
}

/// A `references` answer as places. A lower-bound or truncated count is an
/// incomplete answer, and so are a stale index and a symbol the analyzer has
/// no walk for: no walk is not a proven zero.
pub fn parse_references(answer: &Value) -> Result<ProviderPlaces, ProviderFailure> {
    match answer.get("outcome").and_then(Value::as_str) {
        Some("resolved") => {}
        Some("not_found") => {
            return Ok(ProviderPlaces {
                places: Vec::new(),
                complete: is_fresh(answer),
            })
        }
        Some(_) => {
            return Ok(ProviderPlaces {
                places: Vec::new(),
                complete: false,
            })
        }
        None => return Err(ProviderFailure::Unavailable),
    }
    let references = answer["references"]
        .as_array()
        .ok_or(ProviderFailure::Unavailable)?;
    let places: Vec<ProviderPlace> = references
        .iter()
        .filter_map(|reference| code_place(&reference["location"]))
        .collect();
    let complete = answer["total_is_lower_bound"] == false
        && answer["histogram_truncated"] != true
        && answer["total"].as_u64() == Some(references.len() as u64)
        && places.len() == references.len()
        && is_fresh(answer);
    Ok(ProviderPlaces { places, complete })
}

/// A call-graph answer as the places of the neighbouring methods' names. A
/// capped, dropped or stale answer is incomplete.
pub fn parse_call_neighbours(
    answer: &Value,
    direction: CallDirection,
) -> Result<ProviderPlaces, ProviderFailure> {
    let result = answer.get("result").ok_or(ProviderFailure::Unavailable)?;
    if result.get("error").is_some() {
        return Err(ProviderFailure::Unavailable);
    }
    let nodes = result["nodes"]
        .as_array()
        .ok_or(ProviderFailure::Unavailable)?;
    let places: Vec<ProviderPlace> = nodes
        .iter()
        .filter_map(|node| code_place(&node["location"]))
        .collect();
    let total_field = match direction {
        CallDirection::Incoming => "in_total",
        CallDirection::Outgoing => "out_total",
    };
    let complete = result[total_field].as_u64() == Some(nodes.len() as u64)
        && result["dropped_count"].as_u64() == Some(0)
        && result["connectors_dropped"] == false
        && places.len() == nodes.len()
        && is_fresh(answer);
    Ok(ProviderPlaces { places, complete })
}

impl SymbolProvider for BslAnalyzerSymbols<'_> {
    fn symbol(
        &self,
        query: &SymbolQuery,
        context: &SymbolRequestContext<'_>,
    ) -> Result<Option<ProviderSymbol>, ProviderFailure> {
        Ok(self
            .symbol_info(query, context)?
            .and_then(|(info, fresh)| parse_symbol(&info, fresh)))
    }

    fn references(
        &self,
        query: &SymbolQuery,
        context: &SymbolRequestContext<'_>,
    ) -> Result<ProviderPlaces, ProviderFailure> {
        let arguments = match query {
            SymbolQuery::At { .. } => anchor(query),
            // The reference walk takes the analyzer's qualified name, which
            // only its own name lookup knows.
            SymbolQuery::Named { .. } => {
                let Some(qualified) = self
                    .symbol_info(query, context)?
                    .and_then(|(info, _)| info["address"]["symbol"].as_str().map(str::to_string))
                else {
                    return Ok(ProviderPlaces {
                        places: Vec::new(),
                        complete: false,
                    });
                };
                json!({"symbol": qualified})
            }
        };
        parse_references(&self.call("references", arguments, context)?)
    }

    fn calls(
        &self,
        query: &SymbolQuery,
        direction: CallDirection,
        context: &SymbolRequestContext<'_>,
    ) -> Result<ProviderPlaces, ProviderFailure> {
        let Some((info, _)) = self.symbol_info(query, context)? else {
            return Ok(ProviderPlaces {
                places: Vec::new(),
                complete: false,
            });
        };
        let Some(id) = graph_id(&info).filter(|id| id.starts_with("method/")) else {
            // Not a method: it has no calls.
            return Ok(ProviderPlaces {
                places: Vec::new(),
                complete: true,
            });
        };
        let action = match direction {
            CallDirection::Incoming => "callers",
            CallDirection::Outgoing => "callees",
        };
        let answer = self.call(
            "graph",
            json!({"action": action, "id": id, "edge_kinds": ["call"], "max_nodes": CALL_NEIGHBOURS}),
            context,
        )?;
        parse_call_neighbours(&answer, direction)
    }
}

#[cfg(test)]
mod tests {
    use super::{named_candidate, parse_call_neighbours, parse_references, parse_symbol};
    use serde_json::Value;
    use unica_code_intelligence::symbols::{
        CallDirection, ProviderPlace, SymbolKind, SymbolPosition, SymbolRange,
    };

    /// Answers recorded from `bsl-analyzer 0.2.86` on a small synthetic
    /// workspace: the manager module of `Справочник.Валюты` with a function
    /// `Ф` and a procedure `Вызов` that calls it.
    fn recorded(key: &str) -> Value {
        let all: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/bsl_analyzer/symbols-0.2.86.json"
        ))
        .unwrap();
        all[key]["answer"].clone()
    }

    const MODULE: &str = "Catalogs/Валюты/Ext/ManagerModule.bsl";

    fn at(line: usize, start: usize, end: usize) -> ProviderPlace {
        ProviderPlace::Code {
            path: MODULE.to_string(),
            range: SymbolRange {
                start: SymbolPosition {
                    line,
                    column: start,
                },
                end: SymbolPosition { line, column: end },
            },
        }
    }

    #[test]
    fn recorded_analyzer_answers_become_provider_places() {
        let answer = recorded("symbol_at_call");
        assert!(super::is_fresh(&answer));
        let symbol = parse_symbol(&answer, true).unwrap();
        assert_eq!(symbol.kind, SymbolKind::Method);
        assert_eq!(symbol.definitions, [at(22, 8, 9)]);
        assert!(symbol.signature.is_some());

        let named = recorded("symbol_named");
        assert!(!super::is_fresh(&named), "a capped candidate list");
        let candidate = named_candidate(&named, "Ф", MODULE).unwrap();
        assert_eq!(
            parse_symbol(candidate, false).unwrap().definitions,
            [at(22, 8, 9)]
        );
        assert!(
            named_candidate(&named, "Ф", "CommonModules/Другой/Ext/Module.bsl").is_some(),
            "the only exact candidate is taken"
        );
        assert!(named_candidate(&recorded("symbol_not_found"), "НетТакогоИмени", MODULE).is_none());

        let references = parse_references(&recorded("references_at")).unwrap();
        assert!(references.complete);
        assert_eq!(references.places, [at(22, 8, 9), at(29, 13, 14)]);
        let walkless = parse_references(&recorded("references_unsupported")).unwrap();
        assert!(!walkless.complete, "no walk is not a proven zero");

        let callers = parse_call_neighbours(&recorded("callers"), CallDirection::Incoming).unwrap();
        assert!(callers.complete);
        assert_eq!(callers.places, [at(28, 10, 15)]);
        let callees = parse_call_neighbours(&recorded("callees"), CallDirection::Outgoing).unwrap();
        assert!(callees.places.contains(&at(22, 8, 9)), "{callees:?}");
    }

    #[test]
    fn a_bare_name_shared_by_modules_prefers_its_own_module_and_never_guesses() {
        let answer: Value = serde_json::json!({"candidates": [
            {"match": "exact", "address": {"location": {"path": "CommonModules/А/Ext/Module.bsl", "position_encoding": "utf-16",
                "range": {"start_line": 1, "start_character": 0, "end_line": 1, "end_character": 3}}}},
            {"match": "exact", "address": {"location": {"path": MODULE, "position_encoding": "utf-16",
                "range": {"start_line": 5, "start_character": 0, "end_line": 5, "end_character": 3}}}}
        ]});
        let own = named_candidate(&answer, "ПриЗаписи", MODULE).unwrap();
        assert_eq!(own["address"]["location"]["path"], MODULE);
        assert!(named_candidate(&answer, "ПриЗаписи", "Другой.bsl").is_none());
        assert!(named_candidate(&answer, "Модуль.ПриЗаписи", MODULE).is_none());
    }

    #[test]
    fn a_capped_or_stale_graph_answer_is_incomplete() {
        let mut answer = recorded("references_at");
        answer["freshness"]["completeness"]["status"] = "partial".into();
        assert!(!parse_references(&answer).unwrap().complete);
        let mut answer = recorded("callers");
        answer["freshness"]["stale"] = Value::Bool(true);
        assert!(
            !parse_call_neighbours(&answer, CallDirection::Incoming)
                .unwrap()
                .complete
        );
        let mut answer = recorded("callers");
        answer["result"]["in_total"] = 2.into();
        assert!(
            !parse_call_neighbours(&answer, CallDirection::Incoming)
                .unwrap()
                .complete
        );
    }
}
