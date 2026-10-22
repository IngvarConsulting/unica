//! Module outline built from the BSL file that is on disk right now.
//!
//! ADR-0020: the outline describes the current source as typed data, not an
//! index snapshot or a text report. The only source of truth here is one syntax
//! tree of one file. Nothing in this module reads `bsl_index`, starts a hidden
//! service, or writes workspace state.

use crate::model::CodeIntelligenceContext;
use std::io::Read;
use std::path::{Path, PathBuf};
use unica_bsl::outline::CodeOutlineIdentity;
use unica_bsl::outline::CodeOutlineMethod;
use unica_bsl::outline::CodeOutlineRegion;
use unica_bsl::outline::CodeOutlineResult;
use unica_bsl::outline::CodeOutlineTotals;
use unica_source_contract::discovery::FollowMode;
use unica_work_control::cancellation::{cancelled_error, CancellationToken};
use unica_work_control::provider_deadline::ProviderDeadline;

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
    let reader = context.source_reader();
    let source_root = reader
        .normalize_identity(&context.source_root.path)
        .map_err(|error| format!("could not resolve the selected source root: {error}"))?;
    let module = reader
        .normalize_identity(&source_root.join(Path::new(path)))
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
    let reader = context.source_reader();
    match reader.inspect_input(&module, FollowMode::Follow) {
        Ok(metadata) if metadata.is_dir() => {
            return Err(format!(
                "`{path}` is a directory; `path` expects one BSL module file such as CommonModules/<Имя>/Ext/Module.bsl"
            ));
        }
        Ok(_) => {}
        Err(_) => {
            return Err(format!(
                "module `{path}` does not exist in the selected source root; `path` expects a source-root-relative path to a BSL module file"
            ));
        }
    }
    let mut text = String::new();
    reader
        .open_input(&module)
        .and_then(|mut file| file.read_to_string(&mut text))
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
    use super::{build_result, module_identity, parse_module, ModuleIdentity};
    use serde_json::json;
    use unica_bsl::outline::CodeOutlineResult;

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
