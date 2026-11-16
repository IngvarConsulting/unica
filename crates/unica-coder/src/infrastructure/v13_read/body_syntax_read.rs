//! A Body keeps its text even when the same source has no proven structure.
use super::*;

impl LogicalViewReadAuthority<'_> {
    pub(super) fn body_syntax_observation(
        &self,
        module: &QualifiedAddress,
        text: ModuleText,
        full: Option<ModuleText>,
    ) -> Result<ModuleTextObservation, ViewError> {
        self.read_checkpoint()?;
        let mut diagnostics = Vec::new();
        let mut limits = Vec::new();
        match full.as_ref().unwrap_or(&text) {
            ModuleText::Text(source) => {
                let analysis = unica_bsl::text::analyze_module(source).map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error)
                })?;
                let total = analysis.diagnostics.len();
                // Localize the first defect and name the full count. Parser
                // messages name token kinds, never echo an unbounded token.
                if let Some(first) = analysis.diagnostics.first() {
                    let mut diagnostic = serde_json::to_value(first).map_err(|error| {
                        ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                    })?;
                    diagnostic["severity"] = json!("error");
                    diagnostic["at"] = json!(format!("{module}.Body"));
                    diagnostic["totalSyntaxErrors"] = json!(total);
                    diagnostic["shown"] = json!(1);
                    diagnostics.push(diagnostic);
                }
            }
            ModuleText::TooLarge { bytes, .. } => limits.push(format!(
                "syntax of the complete BSL module was not checked: {bytes} bytes exceeds the bounded syntax input; the selected text remains readable"
            )),
        }
        self.read_checkpoint()?;
        Ok(ModuleTextObservation {
            text,
            diagnostics,
            limits,
        })
    }

    pub(super) fn module_projection_failure(
        &self,
        module: &QualifiedAddress,
        source: Option<&str>,
        message: String,
    ) -> ViewError {
        if let Err(error) = self.read_checkpoint() {
            return error;
        }
        let analysis = source
            .filter(|source| {
                source.len() <= crate::application::v13::source_text::MAX_SOURCE_TEXT_BYTES
            })
            .and_then(|source| unica_bsl::text::analyze_module(source).ok());
        if let Err(error) = self.read_checkpoint() {
            return error;
        }
        if let Some(analysis) = analysis {
            if let Some(first) = analysis.diagnostics.first() {
                let mut error = ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    format!(
                        "BSL structure is unavailable: {} at line {}, column {}",
                        first.message, first.start_line, first.start_column
                    ),
                );
                error.set_next(json!({"tool":"unica.view", "args":{"at":format!("{module}.Body")}, "reason":"Read the exact module text and its syntax diagnostic before restoring a confirmed working state"}));
                return error;
            }
        }
        ViewError::new(RefusalCode::ProviderUnavailable, message)
    }
}
