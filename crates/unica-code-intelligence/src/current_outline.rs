//! Module outline built from the BSL file that is on disk right now.
//!
//! ADR-0020: the outline describes the current source as typed data, not an
//! index snapshot or a text report. The only source of truth here is one syntax
//! tree of one file. Nothing in this module reads `bsl_index`, starts a hidden
//! service, or writes workspace state.

use crate::model::CodeIntelligenceContext;
use std::fs;
use std::path::{Path, PathBuf};
use unica_bsl::outline::CodeOutlineIdentity;
use unica_bsl::outline::CodeOutlineMethod;
use unica_bsl::outline::CodeOutlineRegion;
use unica_bsl::outline::CodeOutlineResult;
use unica_bsl::outline::CodeOutlineTotals;
use unica_work_control::cancellation::{cancelled_error, CancellationToken};
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::paths::normalize_path_identity;

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

/// Builds the typed outline for `path` inside the selected source root.
///
/// Any condition that prevents proving the outline — a read error, a path
/// outside the source root, a parser diagnostic — fails the call instead of
/// publishing a partial tree, because callers use the outline as a map for
/// further reads.
pub fn render_current_source_outline(
    path: &str,
    include_methods: bool,
    context: &CodeIntelligenceContext,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(CodeOutlineResult, PathBuf), String> {
    if cancellation.is_cancelled() {
        return Err(cancelled_error("unica.code.outline stopped before reading"));
    }
    if deadline.is_elapsed() {
        return Err("unica.code.outline provider deadline exceeded before reading".to_string());
    }
    let (module, text) = read_module(path, context)?;
    if cancellation.is_cancelled() {
        return Err(cancelled_error("unica.code.outline stopped after reading"));
    }
    let (methods, regions) = parse_module(&text)?;
    if cancellation.is_cancelled() {
        return Err(cancelled_error("unica.code.outline stopped after parsing"));
    }
    if deadline.is_elapsed() {
        return Err("unica.code.outline provider deadline exceeded after parsing".to_string());
    }
    Ok((
        build_result(
            path,
            &module_identity(path),
            &methods,
            &regions,
            include_methods,
        ),
        module,
    ))
}

/// Resolves the requested module against the selected source root.
///
/// The application port already normalized and contained the argument; this
/// repeats containment against filesystem identity so a symlink resolved after
/// normalization cannot escape the root.
pub fn resolve_module_path(
    path: &str,
    context: &CodeIntelligenceContext,
) -> Result<PathBuf, String> {
    let source_root = normalize_path_identity(&context.source_root.path)
        .map_err(|error| format!("could not resolve the selected source root: {error}"))?;
    let module = normalize_path_identity(&source_root.join(Path::new(path)))
        .map_err(|error| format!("could not resolve module `{path}`: {error}"))?;
    if !module.starts_with(&source_root) {
        return Err(format!(
            "module `{path}` resolves outside the selected source root"
        ));
    }
    Ok(module)
}

fn read_module(path: &str, context: &CodeIntelligenceContext) -> Result<(PathBuf, String), String> {
    let module = resolve_module_path(path, context)?;
    // `path` names one module file. A directory or a missing entry is a caller
    // mistake, and the raw OS message ("Is a directory") does not say which
    // argument is wrong, so both are reported in terms of the contract.
    if module.is_dir() {
        return Err(format!(
            "`{path}` is a directory; `path` expects one BSL module file such as CommonModules/<Имя>/Ext/Module.bsl"
        ));
    }
    if !module.exists() {
        return Err(format!(
            "module `{path}` does not exist in the selected source root; `path` expects a source-root-relative path to a BSL module file"
        ));
    }
    let text = fs::read_to_string(&module)
        .map_err(|error| format!("could not read module `{path}`: {error}"))?;
    Ok((module, text))
}

use unica_bsl_parser::{outline_module as parse_module, OutlineRegion};

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

fn build_result(
    path: &str,
    identity: &ModuleIdentity,
    methods: &[CodeOutlineMethod],
    regions: &[OutlineRegion],
    include_methods: bool,
) -> CodeOutlineResult {
    let (nodes, roots, orphans) = build_tree(regions, methods);
    CodeOutlineResult {
        module: path.to_string(),
        identity: CodeOutlineIdentity {
            category: identity.category.clone(),
            object: identity.object_name.clone(),
            module_type: identity.module_type.clone(),
        },
        totals: CodeOutlineTotals {
            methods: methods.len(),
            exports: methods.iter().filter(|method| method.is_export).count(),
            regions: regions.len(),
            loc: methods
                .iter()
                .map(|method| method.end_line.saturating_sub(method.line) + 1)
                .sum(),
        },
        regions: roots
            .into_iter()
            .map(|root| materialize_region(&nodes, root, methods, include_methods))
            .collect(),
        methods: if include_methods {
            orphans
                .into_iter()
                .map(|index| methods[index].clone())
                .collect()
        } else {
            Vec::new()
        },
    }
}

/// Rebuilds the region tree from flat `[line, end_line]` intervals and hosts each
/// method in the innermost region that contains it. Regions whose intervals cross
/// without nesting degrade to roots, deterministically.
fn build_tree(
    regions: &[OutlineRegion],
    methods: &[CodeOutlineMethod],
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

fn materialize_region(
    nodes: &[RegionNode],
    index: usize,
    methods: &[CodeOutlineMethod],
    include_methods: bool,
) -> CodeOutlineRegion {
    let node = &nodes[index];
    CodeOutlineRegion {
        name: node.region.name.clone(),
        line: node.region.line,
        end_line: node.region.end_line,
        regions: node
            .children
            .iter()
            .map(|child| materialize_region(nodes, *child, methods, include_methods))
            .collect(),
        methods: if include_methods {
            node.methods
                .iter()
                .map(|method| methods[*method].clone())
                .collect()
        } else {
            Vec::new()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_result, module_identity, parse_module, render_current_source_outline, ModuleIdentity,
    };
    use crate::model::CodeIntelligenceContext;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use unica_bsl::outline::CodeOutlineResult;
    use unica_source_contract::workspace::ResolvedSourceRoot;
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_work_control::cancellation::CancellationToken;
    use unica_work_control::provider_deadline::ProviderDeadline;

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

    fn outline(
        workspace: &Workspace,
        path: &str,
        include_methods: bool,
    ) -> Result<CodeOutlineResult, String> {
        render_current_source_outline(
            path,
            include_methods,
            &workspace.context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            &CancellationToken::new(),
        )
        .map(|(result, _)| result)
    }

    fn body(text: &str, include_methods: bool) -> CodeOutlineResult {
        let (methods, regions) = parse_module(text).unwrap();
        build_result(
            "CommonModules/X/Ext/Module.bsl",
            &ModuleIdentity::default(),
            &methods,
            &regions,
            include_methods,
        )
    }

    fn regions_of(text: &str) -> Vec<(Option<String>, usize, Option<usize>)> {
        unica_bsl_parser::outline_regions(text)
            .into_iter()
            .map(|region| (region.name, region.line, region.end_line))
            .collect()
    }

    #[test]
    fn commented_out_declaration_in_a_bsp_header_is_not_a_method() {
        // The regression retained by ADR-0020: the shipped index reads
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

        let result = body(text, true);

        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({
                "module": "CommonModules/X/Ext/Module.bsl",
                "identity": {
                    "category": null,
                    "object": null,
                    "moduleType": null
                },
                "totals": {
                    "methods": 1,
                    "exports": 1,
                    "regions": 1,
                    "loc": 3
                },
                "regions": [{
                    "name": "ПрограммныйИнтерфейс",
                    "line": 1,
                    "endLine": 13,
                    "regions": [],
                    "methods": [{
                        "name": "НастроитьВарианты",
                        "kind": "procedure",
                        "parameters": [{
                            "name": "Настройки",
                            "byValue": false,
                            "defaultValue": null
                        }],
                        "export": true,
                        "line": 9,
                        "endLine": 11
                    }]
                }],
                "methods": []
            })
        );
    }

    #[test]
    fn declaration_inside_a_string_literal_is_not_a_method() {
        let result = body(
            "Процедура Настоящая() Экспорт\n\tТ = \"Процедура Призрак()\";\nКонецПроцедуры\n",
            true,
        );

        assert_eq!(result.methods.len(), 1);
        assert_eq!(result.methods[0].name, "Настоящая");
        assert_eq!(result.methods[0].line, 1);
        assert_eq!(result.methods[0].end_line, 3);
    }

    #[test]
    fn a_region_without_a_name_keeps_its_place_in_the_tree() {
        assert_eq!(
            regions_of("#Область\n#КонецОбласти\n"),
            vec![(None, 1, Some(2))]
        );
        let result = body("#Область\n#КонецОбласти\n", true);
        assert_eq!(result.regions[0].name, None);
        assert_eq!(result.regions[0].line, 1);
        assert_eq!(result.regions[0].end_line, Some(2));
    }

    #[test]
    fn a_default_value_keeps_the_source_text_of_its_expression() {
        // The platform allows only a literal as a default value, and the one
        // literal made of several tokens is a signed number. Joining tokens with
        // a separator turns `-1` into `- 1`, which is neither the source text nor
        // valid BSL, so the source text of the expression node is reported.
        let text = concat!(
            "Процедура П(\n",
            "\tКод = -1,\n",
            "\tДоля = -0.5,\n",
            "\tТекст = \"а, б\",\n",
            "\tДата = '00010101',\n",
            "\tФлаг = Ложь,\n",
            "\tПусто = Неопределено,\n",
            "\tЗнач Обязательный) Экспорт\n",
            "КонецПроцедуры\n",
        );

        let parameters = &body(text, true).methods[0].parameters;

        assert_eq!(
            parameters
                .iter()
                .map(|parameter| (
                    parameter.name.as_str(),
                    parameter.by_value,
                    parameter.default_value.as_deref()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("Код", false, Some("-1")),
                ("Доля", false, Some("-0.5")),
                ("Текст", false, Some("\"а, б\"")),
                ("Дата", false, Some("'00010101'")),
                ("Флаг", false, Some("Ложь")),
                ("Пусто", false, Some("Неопределено")),
                ("Обязательный", true, None),
            ]
        );
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
            serde_json::to_value(body(text, true)).unwrap(),
            json!({
                "module": "CommonModules/X/Ext/Module.bsl",
                "identity": {
                    "category": null,
                    "object": null,
                    "moduleType": null
                },
                "totals": {
                    "methods": 3,
                    "exports": 2,
                    "regions": 2,
                    "loc": 7
                },
                "regions": [{
                    "name": "Внешняя",
                    "line": 3,
                    "endLine": 11,
                    "regions": [{
                        "name": "Внутренняя",
                        "line": 6,
                        "endLine": 10,
                        "regions": [],
                        "methods": [{
                            "name": "Внутренний",
                            "kind": "function",
                            "parameters": [
                                {
                                    "name": "А",
                                    "byValue": true,
                                    "defaultValue": null
                                },
                                {
                                    "name": "Б",
                                    "byValue": false,
                                    "defaultValue": "1"
                                }
                            ],
                            "export": true,
                            "line": 7,
                            "endLine": 9
                        }]
                    }],
                    "methods": [{
                        "name": "Внешний",
                        "kind": "procedure",
                        "parameters": [],
                        "export": true,
                        "line": 4,
                        "endLine": 5
                    }]
                }],
                "methods": [{
                    "name": "Сирота",
                    "kind": "procedure",
                    "parameters": [],
                    "export": false,
                    "line": 1,
                    "endLine": 2
                }]
            })
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

        let result = body(text, false);
        assert_eq!(result.totals.methods, 2);
        assert_eq!(result.totals.exports, 1);
        assert_eq!(result.totals.regions, 1);
        assert_eq!(result.totals.loc, 4);
        assert!(result.methods.is_empty());
        assert!(result.regions[0].methods.is_empty());
    }

    #[test]
    fn bom_and_lf_crlf_cr_line_endings_yield_the_same_coordinates() {
        let base = "#Область Р\nПроцедура П() Экспорт\nКонецПроцедуры\n#КонецОбласти\n";
        let expected = body(base, true);
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
        assert_eq!(first.regions[0].methods[0].name, "Старый");
        assert_eq!(first.regions[0].methods[0].line, 2);
        assert_eq!(first.regions[0].methods[0].end_line, 3);

        fs::write(
            workspace.context.source_root.path.join("CommonModules/X/Ext/Module.bsl"),
            "#Область Р\nПроцедура Старый() Экспорт\nКонецПроцедуры\nПроцедура Новый() Экспорт\nКонецПроцедуры\n#КонецОбласти\n",
        )
        .unwrap();

        let second = outline(&workspace, "CommonModules/X/Ext/Module.bsl", true).unwrap();
        assert_eq!(second.regions[0].methods[1].name, "Новый");
        assert_eq!(second.regions[0].methods[1].line, 4);
        assert_eq!(second.regions[0].methods[1].end_line, 5);
        assert_eq!(second.totals.methods, 2);
        assert_eq!(second.totals.exports, 2);
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
        assert!(error.contains("does not exist"), "{error}");
        assert!(error.contains("BSL module file"), "{error}");
    }

    #[test]
    fn a_path_that_names_a_directory_says_which_argument_is_wrong() {
        // The raw OS message for this mistake is "Is a directory", which names
        // neither the argument nor what it expects. A bare object name lands
        // here too, because the index used to resolve such names and the current
        // source path cannot.
        let workspace = workspace("directory", "CommonModules/X/Ext/Module.bsl", "");

        for path in ["CommonModules/X", "CommonModules/X/Ext"] {
            let error = outline(&workspace, path, true).unwrap_err();

            assert!(error.contains("is a directory"), "{path}: {error}");
            assert!(error.contains("BSL module file"), "{path}: {error}");
            assert!(!error.contains("os error"), "{path}: {error}");
        }
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
    fn no_deadline_outline_reads_source_but_preserves_scope_and_explicit_cancel() {
        let workspace = workspace(
            "no-deadline",
            "CommonModules/X/Ext/Module.bsl",
            "Procedure Post()\nEndProcedure\n",
        );
        let deadline = ProviderDeadline::no_deadline();
        let read = |path: &str, cancellation: &CancellationToken| {
            render_current_source_outline(path, true, &workspace.context, deadline, cancellation)
        };
        let (outline, _) =
            read("CommonModules/X/Ext/Module.bsl", &CancellationToken::new()).unwrap();
        assert_eq!(outline.totals.methods, 1);
        fs::write(
            workspace.root.join("outside.bsl"),
            "Procedure Foreign()\nEndProcedure\n",
        )
        .unwrap();
        assert!(read("../outside.bsl", &CancellationToken::new())
            .unwrap_err()
            .contains("outside the selected source root"));
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(read("CommonModules/X/Ext/Module.bsl", &cancellation)
            .unwrap_err()
            .starts_with("cancelled:"));
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
        assert_eq!(
            serde_json::to_value(build_result(
                "Неизвестно/Что/Ext/Странный.bsl",
                &identity,
                &[],
                &[],
                true,
            ))
            .unwrap(),
            json!({
                "module": "Неизвестно/Что/Ext/Странный.bsl",
                "identity": {
                    "category": null,
                    "object": null,
                    "moduleType": null
                },
                "totals": {
                    "methods": 0,
                    "exports": 0,
                    "regions": 0,
                    "loc": 0
                },
                "regions": [],
                "methods": []
            })
        );
    }
}
