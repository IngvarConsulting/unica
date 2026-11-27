use super::{
    addressed_method, analyze_module, line_end, safe_line_start, Method, StagedCodePostimage,
};
use crate::input::{bad_code_arg, code_projection, CodeDeleteArgs, CodeProjection};
use serde_json::json;
use std::ops::Range;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
use unica_xml_format::text::SourceTextSnapshot;

pub(super) fn plan(
    snapshot: &SourceTextSnapshot,
    args: &CodeDeleteArgs,
    methods: &[Method],
    at_path: &str,
) -> Result<StagedCodePostimage, ApplyPlanError> {
    let Some((module, CodeProjection::Method(name))) = code_projection(&args.at) else {
        return Err(bad_code_arg(
            at_path,
            "code.delete requires a named Method address",
        ));
    };
    let method = addressed_method(methods, &name, at_path)?;
    let refuse = |message| {
        bad_code_arg(at_path, message).with_next(json!({
            "tool": "unica.view",
            "args": { "at": format!("{module}.Body") },
            "reason": "inspect the surrounding module before deciding how to handle the shared boundary",
        }))
    };
    super::reject_lone_cr_line_endings(snapshot)
        .map_err(|_| refuse("lone CR line endings do not prove a complete method boundary"))?;

    let text = snapshot.decoded_text();
    let parsed = unica_bsl_parser::parse(text);
    let comments = parsed.comment_ranges().collect::<Vec<_>>();
    let first_line = safe_line_start(text, method.start);
    let end = line_end(text, method.end);
    let tail = comments
        .iter()
        .find(|comment| comment.start >= method.end && comment.end <= end);
    if method.body_bounds.is_none()
        || !text[first_line..method.start].trim().is_empty()
        || !text[method.end..tail.map_or(end, |comment| comment.start)]
            .trim()
            .is_empty()
        || tail.is_some_and(|comment| !text[comment.end..end].trim().is_empty())
    {
        return Err(refuse(
            "method boundaries share a line with other BSL source",
        ));
    }

    let mut start = first_line;
    for comment in comments
        .iter()
        .rev()
        .filter(|comment| comment.end <= first_line)
    {
        let line = safe_line_start(text, comment.start);
        if line_end(text, comment.end) != start || !text[line..comment.start].trim().is_empty() {
            break;
        }
        start = line;
    }
    let overlaps = |range: &Range<usize>| range.start < end && start < range.end;
    if parsed.conditional_ranges().any(|range| overlaps(&range))
        || parsed
            .region_markers()
            .iter()
            .any(|marker| overlaps(&marker.range))
    {
        return Err(refuse(
            "method overlaps a conditional compilation or region boundary",
        ));
    }
    if !owns_frame(text, &comments, start..end, tail) {
        return Err(refuse(
            "this method does not exclusively own its enclosing comment frame",
        ));
    }

    let mut after = Vec::with_capacity(text.len() - (end - start));
    after.extend_from_slice(&snapshot.raw()[..start]);
    after.extend_from_slice(&snapshot.raw()[end..]);
    let postcondition = || {
        ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "deletion did not preserve the other BSL definitions",
        )
        .at_path(at_path)
    };
    let post = analyze_module(std::str::from_utf8(&after).map_err(|_| postcondition())?)
        .map_err(|_| postcondition())?;
    let remaining = methods.iter().filter(|other| other.start != method.start);
    if !post.diagnostics.is_empty()
        || !post
            .methods
            .iter()
            .map(|method| &method.name)
            .eq(remaining.map(|method| &method.name))
    {
        return Err(postcondition());
    }
    Ok(StagedCodePostimage {
        after,
        no_op: false,
    })
}

fn owns_frame(
    text: &str,
    comments: &[Range<usize>],
    removed: Range<usize>,
    tail: Option<&Range<usize>>,
) -> bool {
    let markers = comments.iter().filter_map(|range| {
        let marker = &text[range.clone()];
        let opens = marker.starts_with("//+");
        (opens || marker.starts_with("//-")).then_some((range, opens))
    });
    let mut depth = 0usize;
    let mut frame = None;
    for (range, opens) in markers {
        if range.start < removed.start {
            depth = if opens {
                depth + 1
            } else {
                depth.saturating_sub(1)
            };
        } else if range.start >= removed.end {
            break;
        } else {
            match (frame, opens) {
                (None, true)
                    if depth == 0 && safe_line_start(text, range.start) == removed.start =>
                {
                    frame = Some(false);
                }
                (Some(false), false) if tail == Some(range) => frame = Some(true),
                _ => return false,
            }
        }
    }
    frame != Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::parse_code_plan_operation;
    use crate::text::plan_code_postimage;

    fn delete(text: &str) -> Result<Vec<u8>, ApplyPlanError> {
        let operation = parse_code_plan_operation(
            "code.delete",
            &json!({"at":"main:CommonModule.Sample.Method.Remove"}),
            0,
            "main",
        )?;
        plan_code_postimage(text.as_bytes(), &operation, 0).map(|plan| plan.after)
    }

    #[test]
    fn delete_method_owns_only_its_lines() {
        let prefix = "\u{feff}// Standalone\r\n\r\n#Область PublicMethods\r\n";
        let removed = "//+ Начало изменения.\r\n// Description\r\n&НаСервере\r\nПроцедура Remove()\r\nКонецПроцедуры //- Конец изменения.\r\n";
        let suffix =
            "\r\n// Neighbor\r\nФункция Keep()\r\nВозврат 1;\r\nКонецФункции\r\n#КонецОбласти";
        assert_eq!(
            delete(&format!("{prefix}{removed}{suffix}")).unwrap(),
            format!("{prefix}{suffix}").as_bytes()
        );

        let only =
            "\u{feff}// Own description\nФункция Remove()\nВозврат 1;\nКонецФункции // Own trailer";
        assert_eq!(delete(only).unwrap(), b"\xef\xbb\xbf");
        let literal =
            "Процедура Remove()\nЗначение = \"//+ literal\";\nКонецПроцедуры // trailer\n";
        assert!(delete(literal).unwrap().is_empty());
        assert_eq!(
            delete("\u{feff}").unwrap_err().kind(),
            ApplyPlanErrorKind::NotFound
        );
    }

    #[test]
    fn delete_method_refuses_shared_lines_and_ambiguous_boundaries() {
        for raw in [
            "//+ shared\nПроцедура Remove()\nКонецПроцедуры\n\nПроцедура Keep()\nКонецПроцедуры //- shared\n",
            "//+ detached\n\nПроцедура Remove()\nКонецПроцедуры //- detached\n",
            "//+ outer\n\n//+ inner\nПроцедура Remove()\nКонецПроцедуры //- inner\n\n//- outer\n",
            "Процедура Remove()\n//+ inside\nКонецПроцедуры //- inside\n",
            "#Если Сервер Тогда\nПроцедура Remove()\nКонецПроцедуры\n#КонецЕсли\n",
            "Процедура Remove()\n#Если Сервер Тогда\n#КонецЕсли\nКонецПроцедуры\n",
            "Процедура Remove() КонецПроцедуры Процедура Keep() КонецПроцедуры\n",
        ] {
            let error = delete(raw).unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{raw:?}: {error:?}");
        }
    }

    #[test]
    fn delete_method_preserves_external_frame_and_separator_comments() {
        let prefix = "//+ Surrounds multiple methods\n\nПроцедура Keep()\nКонецПроцедуры\n\n";
        let removed = "// Own\nПроцедура Remove()\nКонецПроцедуры\n";
        let suffix = "\n//- Surrounds multiple methods\n\n// Module comment";
        assert_eq!(
            delete(&format!("{prefix}{removed}{suffix}")).unwrap(),
            format!("{prefix}{suffix}").as_bytes()
        );
    }
}
