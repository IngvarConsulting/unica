use super::{LineIndex, ParsedModule, QualifiedAddress, RegionProjection};
use std::collections::HashMap;
use unica_bsl::projection::VariableProjection;

pub(super) fn project(
    root: &ParsedModule<'_>,
    lines: &LineIndex<'_>,
    at: &QualifiedAddress,
    regions: &[RegionProjection],
) -> Vec<VariableProjection> {
    let variables = root.variables();
    let mut counts = HashMap::<String, usize>::new();
    for variable in &variables {
        *counts.entry(variable.name.to_lowercase()).or_default() += 1;
    }
    variables
        .into_iter()
        .map(|variable| {
            let line = lines.line_of(variable.keyword.start as u32);
            let region = regions
                .iter()
                .filter(|region| region.line < line && line < region.end_line.unwrap_or(usize::MAX))
                .max_by_key(|region| region.line);
            let mut limits = Vec::new();
            let variable_at = if counts[&variable.name.to_lowercase()] == 1 {
                Some(format!("{at}.Variable.{}", variable.name))
            } else {
                limits.push(format!("variable name `{}` is ambiguous", variable.name));
                None
            };
            if region.is_some_and(|region| !region.addressable) {
                limits.push("the declaring region has no unambiguous address".to_string());
            }
            VariableProjection {
                at: variable_at,
                name: variable.name,
                export: variable.is_export,
                directive: variable.directive,
                line,
                from_line: lines.line_of(variable.declaration.start as u32),
                to_line: lines.line_of(variable.declaration.end.saturating_sub(1) as u32),
                region: region.and_then(|region| region.at.clone()),
                limits,
            }
        })
        .collect()
}
