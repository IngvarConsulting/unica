use crate::{ExtensionAnnotationKind, ParsedModule};
use bsl_syntax::SyntaxKind;
use std::ops::Range;

pub(super) fn effective_source(module: &ParsedModule<'_>) -> Option<String> {
    let source = module.source;
    let mut effective = None;
    let methods = module.methods();
    for method in methods.iter().filter(|method| {
        method.annotations.iter().any(|annotation| {
            annotation.extension_kind == Some(ExtensionAnnotationKind::ChangeAndValidate)
                && annotation
                    .target
                    .as_ref()
                    .is_some_and(|target| !target.is_empty())
        })
    }) {
        let start = method.direct_declaration_end?;
        let end = method.direct_closing_start?;
        for span in inactive_spans(source.get(start..end)?)? {
            let bytes = effective.get_or_insert_with(|| source.as_bytes().to_vec());
            for byte in &mut bytes[start + span.start..start + span.end] {
                if !matches!(*byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
        }
    }
    // Lexical spans start and end at UTF-8 boundaries. Blanking bytes in
    // place keeps original positions available to the structural projection.
    effective.and_then(|bytes| String::from_utf8(bytes).ok())
}

fn inactive_spans(body: &str) -> Option<Vec<Range<usize>>> {
    use SyntaxKind::{PRE_DELETE, PRE_END_DELETE, PRE_END_INSERT, PRE_INSERT};

    let mut open = None;
    let mut spans = Vec::new();
    for token in bsl_parser::line_tokens(body) {
        let kind = token.kind;
        if !matches!(
            kind,
            PRE_DELETE | PRE_END_DELETE | PRE_END_INSERT | PRE_INSERT
        ) {
            continue;
        }
        let span = crate::byte_range(token.range);
        // The upstream lexer also recognizes these names as prefixes.
        // Reject an identifier continuation, leaving adjacent punctuation intact.
        let next = body[span.end..].chars().next();
        if next.is_some_and(|ch| ch == '_' || ch.is_alphanumeric()) {
            return None;
        }
        match kind {
            PRE_DELETE | PRE_INSERT => {
                if open.replace((kind, span.start)).is_some() {
                    return None;
                }
                if kind == PRE_INSERT {
                    spans.push(span);
                }
            }
            PRE_END_DELETE | PRE_END_INSERT => {
                let (opened, start) = open.take()?;
                match (opened, kind) {
                    (PRE_DELETE, PRE_END_DELETE) => {
                        spans.push(start..span.end);
                    }
                    (PRE_INSERT, PRE_END_INSERT) => {
                        spans.push(span);
                    }
                    _ => return None,
                }
            }
            _ => {}
        }
    }
    open.is_none().then_some(spans)
}
