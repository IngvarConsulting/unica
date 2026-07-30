//! Module outline built from the BSL file that is on disk right now.
//!
//! ADR-0020: the outline describes the current source, not an index snapshot, so
//! the only source of truth here is one syntax tree of one file. Nothing in this
//! module reads `bsl_index`, starts a hidden service, or writes workspace state.

use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::{CodeIntelligenceContext, ProviderDeadline};
use crate::infrastructure::source_roots::normalize_path_identity;
use bsl_syntax::ast::{AstNode, FunctionDef, ProcedureDef};
use bsl_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};
use std::fs;
use std::path::Path;

/// Metadata categories of the Designer/platform XML layout. A path component
/// outside this set never becomes a `category`, so an unknown layout reports no
/// object identity instead of an invented one.
const METADATA_CATEGORIES: &[&str] = &[
    "AccountingRegisters",
    "AccumulationRegisters",
    "BusinessProcesses",
    "CalculationRegisters",
    "Catalogs",
    "ChartsOfAccounts",
    "ChartsOfCalculationTypes",
    "ChartsOfCharacteristicTypes",
    "CommonCommands",
    "CommonForms",
    "CommonModules",
    "CommonTemplates",
    "Constants",
    "DataProcessors",
    "DocumentJournals",
    "Documents",
    "Enums",
    "ExchangePlans",
    "ExternalDataSources",
    "FilterCriteria",
    "HTTPServices",
    "InformationRegisters",
    "Reports",
    "Roles",
    "SettingsStorages",
    "Subsystems",
    "Tasks",
    "WebServices",
    "XDTOPackages",
];

/// File name to module kind. The platform writes these names exactly, so a file
/// name outside the table reports no module type.
const MODULE_TYPES: &[(&str, &str)] = &[
    ("commandmodule.bsl", "CommandModule"),
    ("externalconnectionmodule.bsl", "ExternalConnectionModule"),
    ("managedapplicationmodule.bsl", "ManagedApplicationModule"),
    ("managermodule.bsl", "ManagerModule"),
    ("module.bsl", "Module"),
    ("objectmodule.bsl", "ObjectModule"),
    ("ordinaryapplicationmodule.bsl", "OrdinaryApplicationModule"),
    ("recordsetmodule.bsl", "RecordSetModule"),
    ("sessionmodule.bsl", "SessionModule"),
    ("valuemanagermodule.bsl", "ValueManagerModule"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutlineMethod {
    name: String,
    kind: String,
    params: Vec<String>,
    is_export: bool,
    line: usize,
    end_line: usize,
}

impl OutlineMethod {
    fn loc(&self) -> usize {
        self.end_line.saturating_sub(self.line) + 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutlineRegion {
    name: Option<String>,
    line: usize,
    end_line: Option<usize>,
}

impl OutlineRegion {
    /// An unclosed `#Область` spans to the end of the file for containment, but
    /// its reported `end_line` stays unknown.
    fn effective_end(&self) -> usize {
        self.end_line.unwrap_or(usize::MAX)
    }
}

#[derive(Debug)]
struct RegionNode {
    region: OutlineRegion,
    children: Vec<usize>,
    methods: Vec<usize>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ModuleIdentity {
    category: Option<String>,
    object_name: Option<String>,
    module_type: Option<String>,
}

/// Renders the compact outline body for `path` inside the selected source root.
///
/// The grammar of the returned text is the established one; only its source
/// changed. Any condition that prevents proving the outline — a read error, a
/// path outside the source root, a parser diagnostic — fails the call instead of
/// publishing a partial tree, because callers use the outline as a map for
/// further reads.
pub(crate) fn render_current_source_outline(
    path: &str,
    include_methods: bool,
    context: &CodeIntelligenceContext,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<String, String> {
    if cancellation.is_cancelled() {
        return Err(cancelled_error("unica.code.outline stopped before reading"));
    }
    if deadline.remaining().is_zero() {
        return Err("unica.code.outline provider deadline exceeded before reading".to_string());
    }
    let text = read_module(path, context)?;
    if cancellation.is_cancelled() {
        return Err(cancelled_error("unica.code.outline stopped after reading"));
    }
    let (methods, regions) = parse_module(&text)?;
    if cancellation.is_cancelled() {
        return Err(cancelled_error("unica.code.outline stopped after parsing"));
    }
    if deadline.remaining().is_zero() {
        return Err("unica.code.outline provider deadline exceeded after parsing".to_string());
    }
    let body = render(
        path,
        &module_identity(path),
        &methods,
        &regions,
        include_methods,
    );
    Ok(format!("=== bsl-outline ===\n{}", body.trim_end()))
}

fn read_module(path: &str, context: &CodeIntelligenceContext) -> Result<String, String> {
    let source_root = normalize_path_identity(&context.source_root.path)
        .map_err(|error| format!("could not resolve the selected source root: {error}"))?;
    let module = normalize_path_identity(&source_root.join(Path::new(path)))
        .map_err(|error| format!("could not resolve module `{path}`: {error}"))?;
    // The application port already normalized and contained the argument; this
    // repeats containment against filesystem identity right before the read so
    // a symlink resolved after normalization cannot escape the root.
    if !module.starts_with(&source_root) {
        return Err(format!(
            "module `{path}` resolves outside the selected source root"
        ));
    }
    fs::read_to_string(&module).map_err(|error| format!("could not read module `{path}`: {error}"))
}

fn parse_module(text: &str) -> Result<(Vec<OutlineMethod>, Vec<OutlineRegion>), String> {
    if text.len() > u32::MAX as usize {
        return Err("BSL module is too large for the analyzer parser".to_string());
    }
    let parsed = bsl_parser::parse(text);
    if !parsed.errors().is_empty() {
        return Err(format!(
            "BSL module cannot be outlined because the parser reported {} diagnostic(s)",
            parsed.errors().len()
        ));
    }
    let root = parsed.syntax_node();
    let lines = LineIndex::new(text);
    let mut methods = Vec::new();
    let mut markers = Vec::new();
    for node in root.descendants() {
        if node.kind() == SyntaxKind::PRE_REGION_DIR {
            markers.push(node);
            continue;
        }
        if let Some(procedure) = ProcedureDef::cast(node.clone()) {
            methods.push(method_outline(
                procedure.syntax(),
                &lines,
                procedure
                    .name_or_keyword()
                    .map(|token| token.text().to_string()),
                procedure.export_keyword().is_some(),
                SyntaxKind::KW_PROCEDURE,
                SyntaxKind::KW_END_PROCEDURE,
            )?);
        } else if let Some(function) = FunctionDef::cast(node) {
            methods.push(method_outline(
                function.syntax(),
                &lines,
                function
                    .name_or_keyword()
                    .map(|token| token.text().to_string()),
                function.export_keyword().is_some(),
                SyntaxKind::KW_FUNCTION,
                SyntaxKind::KW_END_FUNCTION,
            )?);
        }
    }
    methods.sort_by_key(|method| (method.line, method.end_line));
    Ok((methods, pair_regions(&markers, &lines)))
}

fn method_outline(
    syntax: &SyntaxNode,
    lines: &LineIndex,
    name: Option<String>,
    is_export: bool,
    start_kind: SyntaxKind,
    end_kind: SyntaxKind,
) -> Result<OutlineMethod, String> {
    let name = name.ok_or_else(|| "BSL method is missing a name".to_string())?;
    let start = first_token(syntax, start_kind)
        .ok_or_else(|| format!("BSL method `{name}` is missing its opening keyword"))?;
    let end = first_token(syntax, end_kind)
        .ok_or_else(|| format!("BSL method `{name}` is missing its closing keyword"))?;
    Ok(OutlineMethod {
        name,
        kind: start.text().to_string(),
        params: params(syntax),
        is_export,
        line: lines.line_of(usize::from(start.text_range().start())),
        end_line: lines.line_of(usize::from(end.text_range().start())),
    })
}

fn params(syntax: &SyntaxNode) -> Vec<String> {
    syntax
        .children()
        .find(|child| child.kind() == SyntaxKind::PARAM_LIST)
        .map(|list| {
            list.children()
                .filter(|child| child.kind() == SyntaxKind::PARAM)
                .map(|param| param.text().to_string().trim().to_string())
                .filter(|param| !param.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn first_token(syntax: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    syntax
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == kind)
}

/// Pairs `#Область` with `#КонецОбласти` over the flat directive markers.
///
/// The vendored analyzer parses region directives as flat leaf nodes precisely
/// because a region may overlap a control-flow block without nesting, so the
/// pairing is a stack over source order rather than syntactic nesting.
///
/// The kind of each marker is read from its own token (`PRE_REGION` /
/// `PRE_END_REGION`), never from `PreRegionDir::is_start`/`is_end`/`name`: those
/// helpers compare the node text against four literal spellings, while 1C
/// preprocessor directives are case-insensitive and the lexer accepts a space
/// after `#`. See itrous/bsl-analyzer#11 — on a real vendor configuration the
/// helpers misclassify 19 `#КонецОбласти` markers in 15 modules, each of which
/// would open a region instead of closing one.
fn pair_regions(markers: &[SyntaxNode], lines: &LineIndex) -> Vec<OutlineRegion> {
    let mut regions: Vec<OutlineRegion> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut ordered: Vec<&SyntaxNode> = markers.iter().collect();
    ordered.sort_by_key(|node| node.text_range().start());
    for node in ordered {
        let Some(marker) = region_marker(node) else {
            continue;
        };
        let line = lines.line_of(usize::from(marker.text_range().start()));
        match marker.kind() {
            SyntaxKind::PRE_REGION => {
                regions.push(OutlineRegion {
                    name: region_name(node),
                    line,
                    end_line: None,
                });
                open.push(regions.len() - 1);
            }
            // An unpaired `#КонецОбласти` closes nothing rather than opening a
            // region or failing the call: the file still compiles in 1C.
            _ => {
                if let Some(index) = open.pop() {
                    regions[index].end_line = Some(line);
                }
            }
        }
    }
    regions
}

fn region_marker(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| {
            matches!(
                token.kind(),
                SyntaxKind::PRE_REGION | SyntaxKind::PRE_END_REGION
            )
        })
}

fn region_name(node: &SyntaxNode) -> Option<String> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == SyntaxKind::IDENT)
        .map(|token| token.text().to_string())
        .filter(|name| !name.is_empty())
}

/// One pass over the text yields every line start, so translating byte offsets
/// to line numbers never rescans the beginning of the file per method.
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut starts = vec![0usize];
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\n' => starts.push(index + 1),
                b'\r' => {
                    if bytes.get(index + 1) == Some(&b'\n') {
                        index += 1;
                    }
                    starts.push(index + 1);
                }
                _ => {}
            }
            index += 1;
        }
        Self { starts }
    }

    fn line_of(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(index) => index + 1,
            Err(index) => index,
        }
    }
}

fn module_identity(path: &str) -> ModuleIdentity {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let Some((file_name, directories)) = parts.split_last() else {
        return ModuleIdentity::default();
    };
    let mut identity = ModuleIdentity {
        module_type: MODULE_TYPES
            .iter()
            .find(|(name, _)| file_name.eq_ignore_ascii_case(name))
            .map(|(_, module_type)| (*module_type).to_string()),
        ..ModuleIdentity::default()
    };
    if let Some(index) = directories
        .iter()
        .position(|part| METADATA_CATEGORIES.contains(part))
    {
        identity.category = Some(directories[index].to_string());
        identity.object_name = directories.get(index + 1).map(|part| (*part).to_string());
    }
    identity
}

fn render(
    path: &str,
    identity: &ModuleIdentity,
    methods: &[OutlineMethod],
    regions: &[OutlineRegion],
    include_methods: bool,
) -> String {
    let mut lines = vec![format!("module: {path}")];
    push_identity(&mut lines, "object", identity.object_name.as_deref());
    push_identity(&mut lines, "category", identity.category.as_deref());
    push_identity(&mut lines, "moduleType", identity.module_type.as_deref());
    lines.push(format!(
        "totals: methods={} exports={} regions={} loc={}",
        methods.len(),
        methods.iter().filter(|method| method.is_export).count(),
        regions.len(),
        methods.iter().map(OutlineMethod::loc).sum::<usize>()
    ));
    let (nodes, roots, orphans) = build_tree(regions, methods);
    for root in roots {
        render_region(&nodes, root, 0, methods, include_methods, &mut lines);
    }
    if include_methods {
        for index in orphans {
            lines.push(render_method(&methods[index], 0));
        }
    }
    lines.join("\n")
}

fn push_identity(lines: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        lines.push(format!("{label}: {value}"));
    }
}

/// Rebuilds the region tree from flat `[line, end_line]` intervals and hosts each
/// method in the innermost region that contains it. Regions whose intervals cross
/// without nesting degrade to roots, deterministically.
fn build_tree(
    regions: &[OutlineRegion],
    methods: &[OutlineMethod],
) -> (Vec<RegionNode>, Vec<usize>, Vec<usize>) {
    let mut nodes: Vec<RegionNode> = regions
        .iter()
        .map(|region| RegionNode {
            region: region.clone(),
            children: Vec::new(),
            methods: Vec::new(),
        })
        .collect();
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|index| {
        let region = &nodes[*index].region;
        (region.line, usize::MAX - region.effective_end(), *index)
    });

    let mut roots = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for index in order {
        let (line, end) = {
            let region = &nodes[index].region;
            (region.line, region.effective_end())
        };
        while let Some(top) = stack.last().copied() {
            let parent = &nodes[top].region;
            if parent.line <= line && end <= parent.effective_end() {
                break;
            }
            stack.pop();
        }
        match stack.last().copied() {
            Some(parent) => nodes[parent].children.push(index),
            None => roots.push(index),
        }
        stack.push(index);
    }

    let mut orphans = Vec::new();
    for (method_index, method) in methods.iter().enumerate() {
        let mut host: Option<(usize, (usize, usize, usize))> = None;
        for (index, node) in nodes.iter().enumerate() {
            let region = &node.region;
            if region.line <= method.line && method.end_line <= region.effective_end() {
                let key = (region.line, usize::MAX - region.effective_end(), index);
                if host.is_none_or(|(_, current)| key > current) {
                    host = Some((index, key));
                }
            }
        }
        match host {
            Some((index, _)) => nodes[index].methods.push(method_index),
            None => orphans.push(method_index),
        }
    }
    (nodes, roots, orphans)
}

fn render_region(
    nodes: &[RegionNode],
    index: usize,
    depth: usize,
    methods: &[OutlineMethod],
    include_methods: bool,
    lines: &mut Vec<String>,
) {
    let node = &nodes[index];
    let name = node.region.name.as_deref().unwrap_or("<unnamed>");
    let end_line = node
        .region
        .end_line
        .map(|line| line.to_string())
        .unwrap_or_else(|| "?".to_string());
    lines.push(format!(
        "{}region {name}: {}-{end_line}",
        "  ".repeat(depth),
        node.region.line
    ));
    if include_methods {
        for method in &node.methods {
            lines.push(render_method(&methods[*method], depth + 1));
        }
    }
    for child in &node.children {
        render_region(nodes, *child, depth + 1, methods, include_methods, lines);
    }
}

fn render_method(method: &OutlineMethod, depth: usize) -> String {
    format!(
        "{}{} {}({}){} at {}-{}",
        "  ".repeat(depth),
        method.kind,
        method.name,
        method.params.join(", "),
        if method.is_export { " export" } else { "" },
        method.line,
        method.end_line
    )
}

#[cfg(test)]
mod tests {
    use super::{
        module_identity, pair_regions, parse_module, render, render_current_source_outline,
        LineIndex, ModuleIdentity,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::{CodeIntelligenceContext, ProviderDeadline};
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use bsl_syntax::SyntaxKind;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    struct Workspace {
        root: PathBuf,
        context: CodeIntelligenceContext,
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn workspace(label: &str, path: &str, contents: &str) -> Workspace {
        let root = std::env::temp_dir().join(format!(
            "unica-bsl-outline-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source_root = root.join("src");
        let module = source_root.join(path);
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, contents).unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
        );
        Workspace { root, context }
    }

    fn outline(workspace: &Workspace, path: &str, include_methods: bool) -> Result<String, String> {
        render_current_source_outline(
            path,
            include_methods,
            &workspace.context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            &CancellationToken::new(),
        )
    }

    fn body(text: &str, include_methods: bool) -> String {
        let (methods, regions) = parse_module(text).unwrap();
        render(
            "CommonModules/X/Ext/Module.bsl",
            &ModuleIdentity::default(),
            &methods,
            &regions,
            include_methods,
        )
    }

    fn regions_of(text: &str) -> Vec<(Option<String>, usize, Option<usize>)> {
        let parsed = bsl_parser::parse(text);
        let markers: Vec<_> = parsed
            .syntax_node()
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::PRE_REGION_DIR)
            .collect();
        pair_regions(&markers, &LineIndex::new(text))
            .into_iter()
            .map(|region| (region.name, region.line, region.end_line))
            .collect()
    }

    #[test]
    fn commented_out_declaration_in_a_bsp_header_is_not_a_method() {
        // The regression that motivated ADR-0020: the shipped index reads
        // `// Процедура ОпределитьНастройки(...)` as a real exported method and
        // loses the real one. A syntax tree cannot make that mistake.
        let text = concat!(
            "#Область ПрограммныйИнтерфейс\n",
            "\n",
            "// Пример вызова:\n",
            "//\n",
            "// Процедура ОпределитьНастройки(Форма, Ключ) Экспорт\n",
            "// \t// Код процедуры.\n",
            "// КонецПроцедуры\n",
            "//\n",
            "Процедура НастроитьВарианты(Настройки) Экспорт\n",
            "\tВозврат;\n",
            "КонецПроцедуры\n",
            "\n",
            "#КонецОбласти\n",
        );

        let rendered = body(text, true);

        assert!(
            !rendered.contains("ОпределитьНастройки"),
            "commented-out declaration leaked into the outline:\n{rendered}"
        );
        assert_eq!(
            rendered,
            concat!(
                "module: CommonModules/X/Ext/Module.bsl\n",
                "totals: methods=1 exports=1 regions=1 loc=3\n",
                "region ПрограммныйИнтерфейс: 1-13\n",
                "  Процедура НастроитьВарианты(Настройки) export at 9-11"
            )
        );
    }

    #[test]
    fn declaration_inside_a_string_literal_is_not_a_method() {
        let rendered = body(
            "Процедура Настоящая() Экспорт\n\tТ = \"Процедура Призрак()\";\nКонецПроцедуры\n",
            true,
        );

        assert!(!rendered.contains("Призрак"), "{rendered}");
        assert!(
            rendered.contains("Процедура Настоящая() export at 1-3"),
            "{rendered}"
        );
    }

    #[test]
    fn region_markers_are_classified_by_token_kind_not_by_spelling() {
        // itrous/bsl-analyzer#11: PreRegionDir::is_end() is false for these
        // spellings, so a helper-based tree would open a region here instead of
        // closing one. 1C compiles all of them: directives are case-insensitive.
        for tail in [
            "#КонецОбласти",
            "#Конецобласти",
            "#КОнецОбласти",
            "#КОНЕЦОБЛАСТИ",
        ] {
            let text = format!("#Область Р\nПроцедура П()\nКонецПроцедуры\n{tail}\n");
            assert_eq!(
                regions_of(&text),
                vec![(Some("Р".to_string()), 1, Some(4))],
                "tail {tail}"
            );
        }
        for head in ["#Область Р", "#ОБЛАСТЬ Р", "# Область Р"] {
            let text = format!("{head}\n#КонецОбласти\n");
            assert_eq!(
                regions_of(&text),
                vec![(Some("Р".to_string()), 1, Some(2))],
                "head {head}"
            );
        }
    }

    #[test]
    fn english_and_nested_and_unclosed_and_in_method_regions_are_paired() {
        assert_eq!(
            regions_of("#Region Api\n#EndRegion\n"),
            vec![(Some("Api".to_string()), 1, Some(2))]
        );
        assert_eq!(
            regions_of("#Область Внешняя\n#Область Внутренняя\n#КонецОбласти\n#КонецОбласти\n"),
            vec![
                (Some("Внешняя".to_string()), 1, Some(4)),
                (Some("Внутренняя".to_string()), 2, Some(3)),
            ]
        );
        assert_eq!(
            regions_of("#Область Незакрытая\nПроцедура П()\nКонецПроцедуры\n"),
            vec![(Some("Незакрытая".to_string()), 1, None)]
        );
        // An extra end marker closes nothing instead of failing the call.
        assert_eq!(regions_of("#КонецОбласти\n"), Vec::new());
        assert_eq!(
            regions_of(
                "Процедура П()\n\t#Область Внутри\n\tА = 1;\n\t#КонецОбласти\nКонецПроцедуры\n"
            ),
            vec![(Some("Внутри".to_string()), 2, Some(4))]
        );
    }

    #[test]
    fn a_region_without_a_name_keeps_its_place_in_the_tree() {
        assert_eq!(
            regions_of("#Область\n#КонецОбласти\n"),
            vec![(None, 1, Some(2))]
        );
        assert!(body("#Область\n#КонецОбласти\n", true).contains("region <unnamed>: 1-2"));
    }

    #[test]
    fn methods_land_in_the_innermost_region_and_the_rest_are_orphans() {
        let text = concat!(
            "Процедура Сирота()\n",
            "КонецПроцедуры\n",
            "#Область Внешняя\n",
            "Процедура Внешний() Экспорт\n",
            "КонецПроцедуры\n",
            "#Область Внутренняя\n",
            "Функция Внутренний(Знач А, Б = 1) Экспорт\n",
            "\tВозврат А;\n",
            "КонецФункции\n",
            "#КонецОбласти\n",
            "#КонецОбласти\n",
        );

        assert_eq!(
            body(text, true),
            concat!(
                "module: CommonModules/X/Ext/Module.bsl\n",
                "totals: methods=3 exports=2 regions=2 loc=7\n",
                "region Внешняя: 3-11\n",
                "  Процедура Внешний() export at 4-5\n",
                "  region Внутренняя: 6-10\n",
                "    Функция Внутренний(Знач А, Б = 1) export at 7-9\n",
                "Процедура Сирота() at 1-2"
            )
        );
    }

    #[test]
    fn include_methods_false_drops_leaves_but_keeps_totals() {
        let text = concat!(
            "#Область Р\n",
            "Процедура П() Экспорт\n",
            "КонецПроцедуры\n",
            "#КонецОбласти\n",
            "Процедура Сирота()\n",
            "КонецПроцедуры\n",
        );

        assert_eq!(
            body(text, false),
            concat!(
                "module: CommonModules/X/Ext/Module.bsl\n",
                "totals: methods=2 exports=1 regions=1 loc=4\n",
                "region Р: 1-4"
            )
        );
    }

    #[test]
    fn bom_and_lf_crlf_cr_line_endings_yield_the_same_coordinates() {
        let base = "#Область Р\nПроцедура П() Экспорт\nКонецПроцедуры\n#КонецОбласти\n";
        let expected = concat!(
            "module: CommonModules/X/Ext/Module.bsl\n",
            "totals: methods=1 exports=1 regions=1 loc=2\n",
            "region Р: 1-4\n",
            "  Процедура П() export at 2-3"
        );
        for text in [
            base.to_string(),
            format!("\u{feff}{base}"),
            base.replace('\n', "\r\n"),
            base.replace('\n', "\r"),
            format!("\u{feff}{}", base.replace('\n', "\r\n")),
        ] {
            assert_eq!(body(&text, true), expected, "text {text:?}");
        }
    }

    #[test]
    fn outline_reads_the_file_again_after_it_changed() {
        let workspace = workspace(
            "refresh",
            "CommonModules/X/Ext/Module.bsl",
            "#Область Р\nПроцедура Старый() Экспорт\nКонецПроцедуры\n#КонецОбласти\n",
        );
        let first = outline(&workspace, "CommonModules/X/Ext/Module.bsl", true).unwrap();
        assert!(
            first.contains("Процедура Старый() export at 2-3"),
            "{first}"
        );

        fs::write(
            workspace.context.source_root.path.join("CommonModules/X/Ext/Module.bsl"),
            "#Область Р\nПроцедура Старый() Экспорт\nКонецПроцедуры\nПроцедура Новый() Экспорт\nКонецПроцедуры\n#КонецОбласти\n",
        )
        .unwrap();

        let second = outline(&workspace, "CommonModules/X/Ext/Module.bsl", true).unwrap();
        assert!(
            second.contains("Процедура Новый() export at 4-5"),
            "{second}"
        );
        assert!(second.contains("totals: methods=2 exports=2"), "{second}");
    }

    #[test]
    fn parser_diagnostics_fail_the_call_without_a_partial_outline() {
        let workspace = workspace(
            "broken",
            "CommonModules/X/Ext/Module.bsl",
            "Процедура П(\nКонецПроцедуры\nЕсли Тогда\n",
        );

        let error = outline(&workspace, "CommonModules/X/Ext/Module.bsl", true).unwrap_err();

        assert!(error.contains("parser reported"), "{error}");
        assert!(!error.contains("region"), "{error}");
    }

    #[test]
    fn a_missing_module_fails_with_the_requested_path() {
        let workspace = workspace("missing", "CommonModules/X/Ext/Module.bsl", "");

        let error = outline(&workspace, "CommonModules/Нет/Ext/Module.bsl", true).unwrap_err();

        assert!(
            error.contains("CommonModules/Нет/Ext/Module.bsl"),
            "{error}"
        );
        assert!(error.contains("could not read module"), "{error}");
    }

    #[test]
    fn a_path_outside_the_source_root_is_rejected_before_reading() {
        let workspace = workspace("escape", "CommonModules/X/Ext/Module.bsl", "");
        fs::write(
            workspace.root.join("outside.bsl"),
            "Процедура П()\nКонецПроцедуры\n",
        )
        .unwrap();

        let error = outline(&workspace, "../outside.bsl", true).unwrap_err();

        assert!(
            error.contains("outside the selected source root"),
            "{error}"
        );
    }

    #[test]
    fn cancellation_and_an_exhausted_deadline_stop_before_reading() {
        let workspace = workspace("stop", "CommonModules/X/Ext/Module.bsl", "");
        let cancelled = CancellationToken::new();
        cancelled.cancel();

        let error = render_current_source_outline(
            "CommonModules/X/Ext/Module.bsl",
            true,
            &workspace.context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            &cancelled,
        )
        .unwrap_err();
        assert!(error.contains("stopped before reading"), "{error}");

        let error = render_current_source_outline(
            "CommonModules/X/Ext/Module.bsl",
            true,
            &workspace.context,
            ProviderDeadline::new(Instant::now()),
            &CancellationToken::new(),
        )
        .unwrap_err();
        assert!(error.contains("deadline exceeded"), "{error}");
    }

    #[test]
    fn module_identity_comes_only_from_provable_path_components() {
        let cases = [
            (
                "CommonModules/ОбщегоНазначения/Ext/Module.bsl",
                ("CommonModules", "ОбщегоНазначения", "Module"),
            ),
            (
                "Catalogs/Организации/Ext/ObjectModule.bsl",
                ("Catalogs", "Организации", "ObjectModule"),
            ),
            (
                "Documents/Заказ/Ext/ManagerModule.bsl",
                ("Documents", "Заказ", "ManagerModule"),
            ),
            (
                "AccumulationRegisters/Остатки/Ext/RecordSetModule.bsl",
                ("AccumulationRegisters", "Остатки", "RecordSetModule"),
            ),
            (
                "Catalogs/Организации/Forms/ФормаЭлемента/Ext/Form/Module.bsl",
                ("Catalogs", "Организации", "Module"),
            ),
            (
                "Catalogs/Организации/Commands/Реквизиты/Ext/CommandModule.bsl",
                ("Catalogs", "Организации", "CommandModule"),
            ),
            (
                "WebServices/InterfaceVersion/Ext/Module.bsl",
                ("WebServices", "InterfaceVersion", "Module"),
            ),
        ];
        for (path, (category, object, module_type)) in cases {
            let identity = module_identity(path);
            assert_eq!(identity.category.as_deref(), Some(category), "{path}");
            assert_eq!(identity.object_name.as_deref(), Some(object), "{path}");
            assert_eq!(identity.module_type.as_deref(), Some(module_type), "{path}");
        }
    }

    #[test]
    fn an_unknown_path_family_reports_no_invented_identity() {
        let identity = module_identity("Неизвестно/Что/Ext/Странный.bsl");

        assert_eq!(identity, ModuleIdentity::default());
        let rendered = render("Неизвестно/Что/Ext/Странный.bsl", &identity, &[], &[], true);
        assert_eq!(
            rendered,
            "module: Неизвестно/Что/Ext/Странный.bsl\ntotals: methods=0 exports=0 regions=0 loc=0"
        );
    }

    #[test]
    fn the_line_index_agrees_with_a_naive_scan() {
        for text in [
            "а\nб\r\nв\rг",
            "\u{feff}Процедура П()\r\nКонецПроцедуры\r\n",
            "",
            "\n\n\n",
        ] {
            let index = LineIndex::new(text);
            let bytes = text.as_bytes();
            for offset in 0..=text.len() {
                if !text.is_char_boundary(offset) {
                    continue;
                }
                // A `CRLF` is one terminator, so the offset between its two bytes
                // is not the start of anything and has no line of its own.
                if offset > 0 && bytes[offset - 1] == b'\r' && bytes.get(offset) == Some(&b'\n') {
                    continue;
                }
                let expected = text[..offset]
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .matches('\n')
                    .count()
                    + 1;
                assert_eq!(index.line_of(offset), expected, "text {text:?} at {offset}");
            }
        }
    }
}
