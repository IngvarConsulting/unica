//! Exact projection of pinned current-source BSL findings into diagnostic ranges.
pub fn misplaced_variable_declarations(text: &str) -> Vec<crate::model::DiagnosticRange> {
    unica_bsl_parser::misplaced_variable_declarations(text)
        .into_iter()
        .map(|range| crate::model::DiagnosticRange {
            start_line: range.start_line,
            start_column: range.start_column,
            end_line: range.end_line,
            end_column: range.end_column,
        })
        .collect()
}
