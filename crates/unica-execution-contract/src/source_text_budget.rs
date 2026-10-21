//! One pure Body fit/refusal/range policy, with a required supported-byte bound.
use crate::invocation::DomainResult;
use crate::view_error::ViewError;
use serde_json::{json, Value};
use std::borrow::Cow;
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_refusal::RefusalCode;

/// Запас на конверт ответа с текстом `Body`: итог, адрес дважды, вид,
/// заголовок, словарь операций и ревизия. Настоящий конверт меньше (проверка
/// по проводу); по этому запасу одинаково решаются отказ и пометка
/// `overBudget` в сводке модуля.
const ENVELOPE_RESERVE_BYTES: usize = 2 * 1024;

/// Что именно не поместилось в предел.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OversizedText<'a> {
    Module,
    Method { at: &'a str },
}

/// Отказ для текста сверх предела. Частичного текста нет: `next` ведёт
/// к структурным адресам, по которым модуль читается по смыслу.
pub fn text_too_large(
    module_at: &str,
    subject: OversizedText<'_>,
    bytes: u64,
    max_supported_body_response_bytes: usize,
) -> ViewError {
    let limit = max_supported_body_response_bytes;
    let subject_name = match subject {
        OversizedText::Module => "module",
        OversizedText::Method { .. } => "method",
    };
    let mut error = ViewError::new(
        RefusalCode::ResultTooLarge,
        format!(
            "{subject_name} text is {bytes} bytes, over the {limit}-byte Body limit; read the module by regions and methods"
        ),
    );
    if let OversizedText::Method { at } = subject {
        error.set_next(view_next(at, "подпись, контексты и строки метода"));
    }
    error.set_next(view_next(
        &format!("{module_at}.Region"),
        "области модуля с их методами",
    ));
    error.set_next(view_next(
        &format!("{module_at}.Method"),
        "методы модуля; текст метода — по адресу <метод>.Body",
    ));
    error
}

fn view_next(at: &str, reason: &str) -> Value {
    json!({"tool": "unica.view", "args": {"at": at}, "reason": reason})
}

/// Отказ для текста узла `<владелец>.Body` сверх предела или бюджета:
/// у метода `next` сначала ведёт к самому методу.
pub fn oversized_body(
    body_at: &str,
    bytes: u64,
    max_supported_body_response_bytes: usize,
) -> ViewError {
    let route = QualifiedAddress::parse(body_at).ok().and_then(|at| {
        let segments = at.segments();
        let [.., owner, body] = segments else {
            return None;
        };
        if body.kind() != NodeKind::Body || body.name().is_some() {
            return None;
        }
        let owner_len = segments.len() - 1;
        Some(
            if owner.kind() == NodeKind::Method && owner.name().is_some() {
                (at.prefix(owner_len - 1), Some(at.prefix(owner_len)))
            } else {
                (at.prefix(owner_len), None)
            },
        )
    });
    match route {
        Some((module, Some(method))) => text_too_large(
            &module.to_string(),
            OversizedText::Method {
                at: &method.to_string(),
            },
            bytes,
            max_supported_body_response_bytes,
        ),
        Some((module, None)) => text_too_large(
            &module.to_string(),
            OversizedText::Module,
            bytes,
            max_supported_body_response_bytes,
        ),
        // Без доказанного адреса маршрута нет: пустой адрес в `next` хуже.
        None => ViewError::new(
            RefusalCode::ResultTooLarge,
            format!("source text is {bytes} bytes, over the response limit"),
        ),
    }
}

/// The exact constructor shared by a real Body read and its budget preview.
pub fn body_text_result(
    at: &QualifiedAddress,
    node: serde_json::Map<String, Value>,
    revision: Option<String>,
) -> DomainResult {
    let mut result = DomainResult::success("logical node text resolved");
    result.at = Some(at.to_string());
    result.data = Some(Value::Object(node));
    result.rev = revision;
    result
}

/// A bounded prefix ending at a whole source line. No partial UTF-8/line is
/// advertised when the first line alone exceeds the largest host budget.
pub fn complete_prefix(text: &str, max_supported_body_response_bytes: usize) -> &str {
    if text.len() <= max_supported_body_response_bytes {
        return text;
    }
    text.as_bytes()[..max_supported_body_response_bytes]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or("", |last| &text[..last + 1])
}

fn body_fits(result: &DomainResult, budget: usize) -> bool {
    let Some(text) = result.source_text.as_deref() else {
        return false;
    };
    let mut envelope = result.clone();
    envelope.source_text = None;
    envelope.source_text_read_hint = None;
    let bytes = serde_json::to_vec(&envelope).map_or(usize::MAX, |bytes| bytes.len());
    text_fits(text.len() as u64, budget) && bytes.saturating_add(text.len()) <= budget
}

fn first_body_chunk(result: &DomainResult, budget: usize) -> Option<Value> {
    let hint = result.source_text_read_hint.as_ref()?;
    if hint.from_line == 0 || hint.to_line < hint.from_line {
        return None;
    }
    let prefix = result.source_text.as_deref().or(hint.prefix.as_deref())?;
    let at = QualifiedAddress::parse(result.at.as_deref()?)
        .ok()?
        .without_line_range();
    let node = result.data.as_ref()?.as_object()?;
    let revision = result.rev.clone();
    let preview = |count: usize| -> Option<DomainResult> {
        let end = prefix
            .split_inclusive('\n')
            .take(count)
            .map(str::len)
            .sum::<usize>();
        let to = hint.from_line.checked_add(count.checked_sub(1)?)?;
        if to > hint.to_line {
            return None;
        }
        let ranged = QualifiedAddress::parse(&format!("{at}#L{}-L{to}", hint.from_line)).ok()?;
        let mut data = node.clone();
        data.insert("at".into(), json!(ranged.to_string()));
        data.insert(
            "props".into(),
            json!({"lines": {"state": "range", "from": hint.from_line, "to": to}}),
        );
        if data.contains_key("can") {
            data.insert("can".into(), json!([]));
        }
        let mut candidate = body_text_result(&ranged, data, revision.clone());
        candidate.source_text = Some(prefix[..end].to_string());
        Some(candidate)
    };
    // Binary search complete line prefixes; no per-line envelope allocation
    // and no second source read. The predicate is the real Body fit check.
    let (mut low, mut high) = (
        0,
        prefix
            .split_inclusive('\n')
            .count()
            .min(hint.to_line - hint.from_line + 1),
    );
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if preview(middle).is_some_and(|candidate| body_fits(&candidate, budget)) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let candidate = preview(low)?;
    Some(view_next(
        candidate.at.as_deref()?,
        "первый диапазон полных строк Body в бюджете этого клиента",
    ))
}

fn body_budget_refusal(
    result: &DomainResult,
    text_bytes: usize,
    budget: usize,
    max_supported_body_response_bytes: usize,
) -> DomainResult {
    let error = match result.at.as_deref() {
        Some(at) => oversized_body(at, text_bytes as u64, max_supported_body_response_bytes),
        None => ViewError::new(
            RefusalCode::ResultTooLarge,
            "source text is over the budget",
        ),
    };
    let mut refused = DomainResult::canonical_rejection(
        result.at.clone(),
        RefusalCode::ResultTooLarge,
        format!(
            "the source text is {text_bytes} bytes, over this client's {budget}-byte response budget; read the module by regions and methods",
        ),
    );
    if let Some(chunk) = first_body_chunk(result, budget) {
        refused.next.push(chunk);
    }
    refused.next.extend(error.next().iter().cloned());
    refused
}

/// Largest possible refusal representation for durable admission, independent
/// of the current client. A smaller supported budget shortens the summary's
/// number and can only shorten or remove the first complete-line range.
pub fn maximum_delivery_refusal(
    result: &DomainResult,
    max_supported_body_response_bytes: usize,
) -> Option<DomainResult> {
    if let (true, Some(text)) = (result.ok, result.source_text.as_deref()) {
        return Some(body_budget_refusal(
            result,
            text.len(),
            max_supported_body_response_bytes,
            max_supported_body_response_bytes,
        ));
    }
    if !result.ok && result.source_text_read_hint.is_some() {
        // If a client cannot fit even one complete line, projection may also
        // mark branches overBudget. Include those flags as well as the longest
        // range, even though those two forms need not occur for one client.
        let mut refused =
            apply_response_budget(result, 0, max_supported_body_response_bytes).into_owned();
        refused.source_text_read_hint = None;
        if let Some(chunk) = first_body_chunk(result, max_supported_body_response_bytes) {
            refused.next.insert(0, chunk);
        }
        return Some(refused);
    }
    None
}

/// Ответ в бюджете клиента. Исходный текст, не помещающийся в бюджет вместе
/// с конвертом, заменяется отказом `result_too_large` с маршрутом к структуре
/// модуля; ветвь с размером `bytes`, чей текст не поместится, получает
/// пометку `overBudget`. Сохранённый результат не меняется: бюджет — свойство
/// клиента, а не предмета.
pub fn apply_response_budget(
    result: &DomainResult,
    budget: usize,
    max_supported_body_response_bytes: usize,
) -> Cow<'_, DomainResult> {
    if let (true, Some(text)) = (result.ok, result.source_text.as_deref()) {
        if !body_fits(result, budget) {
            return Cow::Owned(body_budget_refusal(
                result,
                text.len(),
                budget,
                max_supported_body_response_bytes,
            ));
        }
        return Cow::Borrowed(result);
    }
    if !result.ok && result.source_text_read_hint.is_some() {
        if let Some(chunk) = first_body_chunk(result, budget) {
            let mut refused = result.clone();
            refused.next.insert(0, chunk);
            return Cow::Owned(refused);
        }
    }
    let over = |branch: &Value| {
        branch
            .get("bytes")
            .and_then(Value::as_u64)
            .is_some_and(|bytes| !text_fits(bytes, budget))
    };
    let marked = result
        .data
        .as_ref()
        .and_then(|data| data.get("branches"))
        .and_then(Value::as_array)
        .is_some_and(|branches| branches.iter().any(over));
    if !marked {
        return Cow::Borrowed(result);
    }
    let mut result = result.clone();
    if let Some(branches) = result
        .data
        .as_mut()
        .and_then(|data| data.get_mut("branches"))
        .and_then(Value::as_array_mut)
    {
        for branch in branches.iter_mut() {
            if over(branch) {
                branch["overBudget"] = Value::Bool(true);
            }
        }
    }
    Cow::Owned(result)
}

/// Текст этого размера вместе с запасом на конверт помещается в бюджет.
fn text_fits(bytes: u64, budget: usize) -> bool {
    bytes.saturating_add(ENVELOPE_RESERVE_BYTES as u64) <= budget as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation::SourceTextReadHint;
    const MAX_SOURCE_TEXT_BYTES: usize = 73_728; // Explicit fixture limit, not production policy.
    fn body(text: &str, first: usize) -> DomainResult {
        let at = QualifiedAddress::parse("main:CommonModule.Probe.Method.Ping.Body").unwrap();
        let mut result = body_text_result(&at, serde_json::from_value(json!({"at":at.to_string(),"kind":"Body","title":"Body","can":[{"op":"code.replace"}]})).unwrap(), Some("revision".into()));
        result.source_text = Some(text.into());
        result.source_text_read_hint = Some(Box::new(SourceTextReadHint {
            from_line: first,
            to_line: first + text.lines().count().saturating_sub(1),
            prefix: None,
        }));
        result
    }

    #[test]
    fn response_budget_refuses_text_with_its_envelope_and_marks_oversized_branches() {
        use crate::invocation::DomainResult;
        use serde_json::json;
        let at = "main:CommonModule.Пример.Method.Большой.Body";
        let mut result = DomainResult::success("logical node text resolved");
        result.at = Some(at.into());
        result.data = Some(json!({"at": at, "kind": "Body", "title": "Body"}));
        // Отказ и пометка решаются одним запасом на конверт: 2 КиБ. При
        // бюджете 16 КиБ проходит текст до 14 336 байт включительно.
        result.source_text = Some("a".repeat(14_336));
        assert!(
            super::apply_response_budget(&result, 16_384, MAX_SOURCE_TEXT_BYTES).as_ref()
                == &result
        );
        result.source_text = Some("a".repeat(14_337));
        let refused =
            super::apply_response_budget(&result, 16_384, MAX_SOURCE_TEXT_BYTES).into_owned();
        assert!(!refused.ok);
        assert!(refused.source_text.is_none());
        assert_eq!(refused.diagnostics[0]["code"], "result_too_large");
        let targets = refused
            .next
            .iter()
            .map(|next| next["args"]["at"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            targets,
            [
                "main:CommonModule.Пример.Method.Большой",
                "main:CommonModule.Пример.Region",
                "main:CommonModule.Пример.Method"
            ]
        );

        let mut summary = DomainResult::success("logical node resolved");
        summary.data = Some(json!({"branches": [
            {"at": "m.Method", "count": 3},
            {"at": "m.Body", "count": 900, "bytes": 30_000}
        ]}));
        let wide = super::apply_response_budget(&summary, 47_104, MAX_SOURCE_TEXT_BYTES);
        assert!(wide.as_ref() == &summary);
        let default =
            super::apply_response_budget(&summary, 16_384, MAX_SOURCE_TEXT_BYTES).into_owned();
        let branches = default.data.unwrap()["branches"].clone();
        assert_eq!(branches[1]["overBudget"], true);
        assert!(branches[0].get("overBudget").is_none());
    }

    #[test]
    fn overbudget_mark_and_refusal_share_one_boundary() {
        use crate::invocation::DomainResult;
        use serde_json::json;
        let branch = |bytes: u64| {
            let mut summary = DomainResult::success("logical node resolved");
            summary.data =
                Some(json!({"branches": [{"at": "m.Body", "count": 1, "bytes": bytes}]}));
            super::apply_response_budget(&summary, 16_384, MAX_SOURCE_TEXT_BYTES)
                .into_owned()
                .data
                .unwrap()["branches"][0]
                .get("overBudget")
                .cloned()
        };
        assert_eq!(branch(14_336), None);
        assert_eq!(branch(14_337), Some(json!(true)));
    }

    #[test]
    fn oversized_body_routes_by_address_segments_not_by_text() {
        // Объект с именем `Method` не делает модуль методом.
        let error = super::oversized_body(
            "main:Catalog.Method.Module.Object.Body",
            1,
            MAX_SOURCE_TEXT_BYTES,
        );
        let targets = error
            .next()
            .iter()
            .map(|next| next["args"]["at"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            targets,
            [
                "main:Catalog.Method.Module.Object.Region",
                "main:Catalog.Method.Module.Object.Method"
            ]
        );
        // Без доказанного адреса маршрута нет.
        assert!(
            super::oversized_body("not an address", 1, MAX_SOURCE_TEXT_BYTES)
                .next()
                .is_empty()
        );
        let mut result = crate::invocation::DomainResult::success("text");
        result.source_text = Some("a".repeat(20_000));
        let refused =
            super::apply_response_budget(&result, 16_384, MAX_SOURCE_TEXT_BYTES).into_owned();
        assert_eq!(refused.diagnostics[0]["code"], "result_too_large");
        assert!(refused.next.is_empty());
    }

    #[test]
    fn oversized_method_refusal_routes_to_the_method_and_module_structure() {
        let module = "main:CommonModule.Пример";
        let method = format!("{module}.Method.Большой");
        let error = text_too_large(
            module,
            OversizedText::Method { at: &method },
            MAX_SOURCE_TEXT_BYTES as u64 + 1,
            MAX_SOURCE_TEXT_BYTES,
        );
        assert_eq!(error.code(), RefusalCode::ResultTooLarge);
        let targets = error
            .next()
            .iter()
            .map(|next| next["args"]["at"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            targets,
            [
                method,
                format!("{module}.Region"),
                format!("{module}.Method")
            ]
        );
    }

    #[test]
    fn admission_refusal_bounds_client_specific_range_hints() {
        use crate::result_delivery::error_delivery_text;

        fn delivery_bytes(result: &DomainResult) -> usize {
            let mut public = result.clone();
            public.source_text_read_hint = None;
            let text = error_delivery_text(&public).unwrap();
            serde_json::to_vec(&public).unwrap().len()
                + serde_json::to_vec(&text.diagnostic).unwrap().len()
                + text
                    .recovery
                    .map_or(0, |text| serde_json::to_vec(&text).unwrap().len())
        }

        let text = format!("{}\n", "ю🙂".repeat(30)).repeat(500);
        for first in [9, 99, 999] {
            let original = body(&text, first);
            let mut stored_refusal = original.clone();
            stored_refusal.ok = false;
            stored_refusal.source_text = None;
            stored_refusal
                .source_text_read_hint
                .as_mut()
                .unwrap()
                .prefix = Some(complete_prefix(&text, MAX_SOURCE_TEXT_BYTES).to_owned());
            stored_refusal.next = oversized_body(
                original.at.as_deref().unwrap(),
                text.len() as u64,
                MAX_SOURCE_TEXT_BYTES,
            )
            .next()
            .to_vec();
            stored_refusal.data.as_mut().unwrap()["branches"] = json!([{"bytes": 1}]);
            for stored in [original, stored_refusal] {
                let upper = maximum_delivery_refusal(&stored, MAX_SOURCE_TEXT_BYTES).unwrap();
                assert!(!upper.ok);
                assert!(upper.source_text_read_hint.is_none());
                let upper_bytes = delivery_bytes(&upper);
                for budget in [
                    1_024,
                    1_025,
                    4_095,
                    4_096,
                    16_384,
                    16_385,
                    MAX_SOURCE_TEXT_BYTES,
                ] {
                    let actual = apply_response_budget(&stored, budget, MAX_SOURCE_TEXT_BYTES);
                    assert!(
                        !actual.ok,
                        "fixture must exercise refusal at budget {budget}"
                    );
                    assert!(
                        delivery_bytes(&actual) <= upper_bytes,
                        "budget {budget}, first line {first} outgrew admission"
                    );
                }
                crate::result_delivery::validate_result_delivery(&stored, MAX_SOURCE_TEXT_BYTES)
                    .unwrap();
            }
        }
    }

    #[test]
    fn continuation_fits_the_real_ranged_envelope_at_each_host_budget() {
        let line = format!("{}\r\n", "ю".repeat(100));
        let text = format!("{}конец", line.repeat(40));
        let original = body(&text, 97);
        for budget in [2500, 3000, 4000, 8192] {
            let refused =
                apply_response_budget(&original, budget, MAX_SOURCE_TEXT_BYTES).into_owned();
            assert!(!refused.ok, "{budget}");
            let first = &refused.next[0]["args"]["at"];
            let at = QualifiedAddress::parse(first.as_str().unwrap()).unwrap();
            let range = at.line_range().unwrap();
            assert_eq!(range.from, 97);
            let prefix = text
                .split_inclusive('\n')
                .take(range.to - range.from + 1)
                .collect::<String>();
            let mut node = original.data.clone().unwrap().as_object().unwrap().clone();
            node.insert("at".into(), json!(at.to_string()));
            node.insert("can".into(), json!([]));
            node.insert(
                "props".into(),
                json!({"lines":{"state":"range","from":range.from,"to":range.to}}),
            );
            let mut actual = body_text_result(&at, node, Some("revision".into()));
            actual.source_text = Some(prefix);
            assert!(body_fits(&actual, budget));
            assert!(apply_response_budget(&actual, budget, MAX_SOURCE_TEXT_BYTES).ok);
            let next_text = text
                .split_inclusive('\n')
                .take(range.to - range.from + 2)
                .collect::<String>();
            let next_at = QualifiedAddress::parse(&format!(
                "{}#L{}-L{}",
                at.without_line_range(),
                range.from,
                range.to + 1
            ))
            .unwrap();
            actual.at = Some(next_at.to_string());
            actual.data.as_mut().unwrap()["at"] = json!(next_at.to_string());
            actual.data.as_mut().unwrap()["props"]["lines"]["to"] = json!(range.to + 1);
            actual.source_text = Some(next_text);
            assert!(
                !body_fits(&actual, budget),
                "must be the largest first prefix for {budget}"
            );
        }
        assert_eq!(original.source_text.as_deref(), Some(text.as_str()));
        assert_eq!(
            original.source_text_read_hint.as_ref().unwrap().from_line,
            97
        );
    }

    #[test]
    fn no_first_fitting_line_means_no_false_range_continuation() {
        for text in ["a".repeat(4096), format!("{}\nshort\n", "я".repeat(4096))] {
            let result = body(&text, 12);
            let refused = apply_response_budget(&result, 2048, MAX_SOURCE_TEXT_BYTES).into_owned();
            assert!(!refused.ok);
            assert!(refused
                .next
                .iter()
                .all(|next| !next["args"]["at"].as_str().unwrap_or("").contains("#L")));
        }
        let empty = body("", 1);
        assert!(first_body_chunk(&empty, 2048).is_none());
    }
}
