//! Syntax evidence for a complete BSL source, shared by readers and strict projections.
use crate::text::{validation_diagnostic, ValidationDiagnostic};
use unica_bsl_parser::ParsedModule;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SyntaxObservation {
    /// No complete source was analyzed; this is not a successful parse.
    #[default]
    Unassessed,
    Valid,
    Invalid {
        diagnostics: Vec<ValidationDiagnostic>,
        omitted: usize,
    },
}
impl SyntaxObservation {
    pub fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid { .. })
    }
    pub fn diagnostic_messages(&self, module_at: &str) -> Vec<String> {
        match self {
            Self::Invalid {
                diagnostics,
                omitted,
            } => {
                // Keep evidence inside the Body envelope reserve, even for many
                // diagnostics or long source tokens/addresses. The source itself
                // remains available independently of this bounded explanation.
                const DETAIL_BYTES: usize = 1280;
                let mut messages = Vec::new();
                let mut bytes = 2; // JSON array brackets.
                let mut hidden = *omitted;
                for (index, diagnostic) in diagnostics.iter().enumerate() {
                    let message = format!(
                        "line {}, column {}: {} ({module_at})",
                        diagnostic.start_line, diagnostic.start_column, diagnostic.message
                    );
                    let encoded =
                        |text: &str| serde_json::to_vec(text).expect("strings serialize").len();
                    let remaining = DETAIL_BYTES - bytes;
                    if encoded(&message) < remaining {
                        bytes += encoded(&message) + 1;
                        messages.push(message);
                    } else {
                        let mut end = remaining.saturating_sub(64).min(message.len());
                        while !message.is_char_boundary(end) {
                            end -= 1;
                        }
                        while end > 0 {
                            let shortened =
                                format!("{} [diagnostic text truncated]", &message[..end]);
                            if encoded(&shortened) < remaining {
                                messages.push(shortened);
                                break;
                            }
                            end /= 2;
                            while !message.is_char_boundary(end) {
                                end -= 1;
                            }
                        }
                        hidden += diagnostics.len() - index;
                        break;
                    }
                }
                // The fixed omission notice fits the remaining envelope reserve.
                if hidden > 0 {
                    messages.push(format!(
                        "{hidden} additional or truncated BSL syntax diagnostic(s) omitted"
                    ));
                }
                messages
            }
            _ => Vec::new(),
        }
    }
}
/// Parse once and retain both the tree and the exact diagnostic locations.
pub fn parse_source(source: &str) -> Result<(ParsedModule<'_>, SyntaxObservation), String> {
    if source.len() > u32::MAX as usize {
        return Err("BSL module is too large for the analyzer parser".into());
    }
    let parsed = unica_bsl_parser::parse(source);
    let diagnostics = parsed
        .errors()
        .iter()
        .take(5)
        .map(|error| validation_diagnostic(source, error))
        .collect::<Result<Vec<_>, _>>()?;
    let observation = if diagnostics.is_empty() {
        SyntaxObservation::Valid
    } else {
        SyntaxObservation::Invalid {
            omitted: parsed.errors().len().saturating_sub(diagnostics.len()),
            diagnostics,
        }
    };
    Ok((parsed, observation))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn syntax_read_diagnostics_bound_encoded_bytes_and_name_omissions() {
        let (_, invalid) = parse_source(&"Процедура А()\r\n".repeat(300)).unwrap();
        let SyntaxObservation::Invalid {
            diagnostics,
            omitted,
        } = invalid
        else {
            panic!("invalid syntax expected")
        };
        assert_eq!(diagnostics.len(), 5);
        assert!(omitted > 0);
        let mut diagnostic = diagnostics[0].clone();
        diagnostic.message = "\"\\\n\u{0001}Ю".repeat(500);
        let invalid = SyntaxObservation::Invalid {
            diagnostics: vec![diagnostic; 5],
            omitted: usize::MAX - 5,
        };
        let messages =
            invalid.diagnostic_messages(&"main:CommonModule.ОченьДлинноеИмя".repeat(500));
        assert!(serde_json::to_vec(&messages).unwrap().len() < 1536);
        assert!(messages[0].starts_with("line "));
        assert!(messages.last().unwrap().contains("omitted"));
    }
}
