//! The analyzer behind Unica's symbol port. Its `symbol_info`, `references`
//! and call `graph` answer in file ranges and graph ids; this adapter turns
//! them into provider places and never lets the analyzer's own names through.

use serde_json::{json, Value};
use unica_code_intelligence::model::CodeIntelligenceContext;
use unica_code_intelligence::symbols::{
    CallDirection, ProviderPlace, ProviderPlaces, ProviderSymbol, SymbolKind, SymbolPosition,
    SymbolProvider, SymbolQuery, SymbolRange, SymbolRequestContext,
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

    fn call(
        &self,
        tool: &'static str,
        arguments: Value,
        context: &SymbolRequestContext<'_>,
    ) -> Result<Value, String> {
        if context.cancellation.is_cancelled() {
            return Err("symbol request was cancelled".to_string());
        }
        let timeout = context
            .deadline
            .finite_remaining()
            .map_err(|error| error.to_string())?;
        let output = self.client.call(
            tool,
            &self.context,
            arguments,
            timeout,
            context.cancellation,
        )?;
        serde_json::from_str(output.result_text.trim())
            .map_err(|error| format!("invalid {tool} answer from the analyzer: {error}"))
    }

    /// The analyzer's `symbol_info` answer for a query: a position, or the
    /// exact candidate of a name.
    fn symbol_info(
        &self,
        query: &SymbolQuery,
        context: &SymbolRequestContext<'_>,
    ) -> Result<Option<Value>, String> {
        let answer = self.call("symbol_info", anchor(query), context)?;
        match answer.get("status").and_then(Value::as_str) {
            Some("ok") => Ok(Some(answer)),
            _ => Ok(answer["candidates"]
                .as_array()
                .and_then(|candidates| {
                    candidates.iter().find(|candidate| {
                        candidate["match"] == "exact"
                            && candidate["address"]["location"].is_object()
                    })
                })
                .cloned()),
        }
    }
}

fn anchor(query: &SymbolQuery) -> Value {
    match query {
        SymbolQuery::At { path, position } => {
            json!({"path": path, "line": position.line, "column": position.column})
        }
        SymbolQuery::Named { name } => json!({"symbol": name}),
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
    if kind.contains("metadata") || kind == "metadata_object" || kind == "metadata_member" {
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

/// A `symbol_info` answer, or an exact name candidate, as a provider symbol.
pub fn parse_symbol(info: &Value) -> Option<ProviderSymbol> {
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
    })
}

/// A `references` answer as places; a lower-bound or truncated count is an
/// incomplete answer, and so is a symbol the analyzer has no reference walk for.
pub fn parse_references(answer: &Value) -> Result<ProviderPlaces, String> {
    match answer.get("outcome").and_then(Value::as_str) {
        Some("resolved") => {}
        Some("not_found") => {
            return Ok(ProviderPlaces {
                places: Vec::new(),
                complete: true,
            })
        }
        Some(_) => {
            return Ok(ProviderPlaces {
                places: Vec::new(),
                complete: false,
            })
        }
        None => return Err("references answer has no outcome".to_string()),
    }
    let references = answer["references"]
        .as_array()
        .ok_or_else(|| "references answer has no reference list".to_string())?;
    let places: Vec<ProviderPlace> = references
        .iter()
        .filter_map(|reference| code_place(&reference["location"]))
        .collect();
    let complete = answer["total_is_lower_bound"] == false
        && answer["histogram_truncated"] != true
        && answer["total"].as_u64() == Some(references.len() as u64)
        && places.len() == references.len();
    Ok(ProviderPlaces { places, complete })
}

/// A call-graph answer as the places of the neighbouring methods' names.
pub fn parse_call_neighbours(
    answer: &Value,
    direction: CallDirection,
) -> Result<ProviderPlaces, String> {
    let result = answer
        .get("result")
        .ok_or_else(|| "call graph answer has no result".to_string())?;
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    let nodes = result["nodes"]
        .as_array()
        .ok_or_else(|| "call graph answer has no node list".to_string())?;
    let places: Vec<ProviderPlace> = nodes
        .iter()
        .filter_map(|node| code_place(&node["location"]))
        .collect();
    let total_field = match direction {
        CallDirection::Incoming => "in_total",
        CallDirection::Outgoing => "out_total",
    };
    let complete = result[total_field].as_u64() == Some(nodes.len() as u64)
        && places.len() == nodes.len()
        && answer.get("stale") != Some(&Value::Bool(true));
    Ok(ProviderPlaces { places, complete })
}

impl SymbolProvider for BslAnalyzerSymbols<'_> {
    fn symbol(
        &self,
        query: &SymbolQuery,
        context: &SymbolRequestContext<'_>,
    ) -> Result<Option<ProviderSymbol>, String> {
        Ok(self
            .symbol_info(query, context)?
            .as_ref()
            .and_then(parse_symbol))
    }

    fn references(
        &self,
        query: &SymbolQuery,
        context: &SymbolRequestContext<'_>,
    ) -> Result<ProviderPlaces, String> {
        let arguments = match query {
            SymbolQuery::At { .. } => anchor(query),
            // The reference walk takes the analyzer's qualified name, which
            // only its own name lookup knows.
            SymbolQuery::Named { .. } => {
                let Some(qualified) = self
                    .symbol_info(query, context)?
                    .and_then(|info| info["address"]["symbol"].as_str().map(str::to_string))
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
    ) -> Result<ProviderPlaces, String> {
        let Some(id) = self
            .symbol_info(query, context)?
            .as_ref()
            .and_then(graph_id)
            .filter(|id| id.starts_with("method/"))
            .map(str::to_string)
        else {
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
    use super::{parse_call_neighbours, parse_references, parse_symbol};
    use serde_json::json;
    use unica_code_intelligence::symbols::{CallDirection, ProviderPlace, SymbolKind};

    fn location(line: u64, start: u64, end: u64) -> serde_json::Value {
        json!({
            "path": "Catalogs/Валюты/Ext/ManagerModule.bsl",
            "position_encoding": "utf-16",
            "range": {"start_line": line, "start_character": start, "end_line": line, "end_character": end},
            "root_id": "",
            "schema_version": "1"
        })
    }

    /// Answer shapes measured on `bsl-analyzer 0.2.86`.
    #[test]
    fn analyzer_answers_become_provider_places() {
        let symbol = parse_symbol(&json!({
            "status": "ok", "symbol": "Ф", "kind": "method",
            "signature": "Функция Ф()", "return_type": "РезультатЗапроса | Неопределено",
            "graph_id": "method/manager/Catalog/Валюты/Ф",
            "definitions": [{"location": location(22, 8, 9), "role": "base"}]
        }))
        .unwrap();
        assert_eq!(symbol.kind, SymbolKind::Method);
        assert_eq!(
            symbol.return_type.as_deref(),
            Some("РезультатЗапроса | Неопределено")
        );
        assert!(
            matches!(&symbol.definitions[..], [ProviderPlace::Code { range, .. }] if range.start.line == 22)
        );

        let candidate = parse_symbol(&json!({
            "address": {"graph_id": "method/manager/Catalog/Валюты/Ф", "location": location(22, 8, 9), "symbol": "Справочник.Валюты.Ф"},
            "category": "module_method", "display": "Ф", "match": "exact"
        }))
        .unwrap();
        assert_eq!(candidate.definitions.len(), 1);

        let attribute = parse_symbol(&json!({
            "status": "ok", "symbol": "Код", "kind": "metadata member",
            "graph_id": "attribute/Catalog/Валюты/Наценка", "definitions": []
        }))
        .unwrap();
        assert_eq!(
            attribute.definitions,
            [ProviderPlace::Metadata {
                kind: "Catalog".to_string(),
                object: "Валюты".to_string(),
                member: Some("Наценка".to_string())
            }]
        );

        let references = parse_references(&json!({
            "outcome": "resolved", "total": 2, "total_is_lower_bound": false, "histogram_truncated": false,
            "references": [{"kind": "declaration", "location": location(22, 8, 9)}, {"kind": "call", "location": location(29, 13, 14)}]
        }))
        .unwrap();
        assert!(references.complete);
        assert_eq!(references.places.len(), 2);
        let walkless = parse_references(&json!({"outcome": "unsupported_symbol"})).unwrap();
        assert!(!walkless.complete, "no walk is not a proven zero");

        let callers = parse_call_neighbours(&json!({"stale": false, "result": {
            "in_total": 2, "nodes": [{"id": "method/manager/Catalog/Валюты/Вызов", "location": location(28, 10, 15)}]
        }}), CallDirection::Incoming)
        .unwrap();
        assert!(!callers.complete, "a capped page is incomplete");
        assert_eq!(callers.places.len(), 1);
    }
}
