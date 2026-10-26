use super::*;
use crate::application::lsp::uri::WorkspaceFolder;
use crate::infrastructure::lsp_documents::{LspDocuments, SourceSetLayout, SpaceLayout};
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use unica_bsl::profile::PlatformProfile;
use unica_source_contract::SourceSetKind;
use unica_workspace::documents::DocumentOverlay;

const MODULE: &str = "#Область ПрограммныйИнтерфейс

// Складывает 😀 числа.
// Второе необязательно.
&НаСервере
Функция Сложить(Знач А, Б = 1) Экспорт
\tЗапрос = \"ВЫБРАТЬ
\t|  1\";
\tЕсли А > 0 Тогда
\t\tВозврат Сложить(А, );
\tКонецЕсли;
\tВозврат ОбщМод.Метод(А, Б);
КонецФункции

#КонецОбласти
";

fn at(text: &str, needle: &str) -> Utf16Position {
    let lines = LineIndex::new(text);
    lines.position(text, text.find(needle).unwrap())
}

fn after(text: &str, needle: &str) -> Utf16Position {
    let lines = LineIndex::new(text);
    lines.position(text, text.find(needle).unwrap() + needle.len())
}

#[test]
fn document_symbols_are_a_tree_in_utf16_positions() {
    let symbols = document_symbols(MODULE);
    let region = &symbols[0];
    assert_eq!(region["name"], "ПрограммныйИнтерфейс");
    assert_eq!(region["kind"], SYMBOL_NAMESPACE);
    let add = &region["children"][0];
    assert_eq!(add["name"], "Сложить");
    assert_eq!(add["kind"], SYMBOL_FUNCTION);
    assert_eq!(add["detail"], "(Знач А, Б = 1) Экспорт");
    // «Функция » is eight UTF-16 units.
    assert_eq!(
        add["selectionRange"],
        json!({"start": {"line": 5, "character": 8}, "end": {"line": 5, "character": 15}})
    );
    assert_eq!(add["range"]["start"], json!({"line": 4, "character": 0}));
    assert_eq!(add["range"]["end"]["line"], 12);
}

#[test]
fn folds_keep_end_keywords_visible() {
    assert_eq!(
        folding_ranges(MODULE),
        json!([
            {"startLine": 0, "endLine": 13, "kind": "region"},
            {"startLine": 2, "endLine": 3, "kind": "comment"},
            {"startLine": 5, "endLine": 11},
            {"startLine": 8, "endLine": 9},
        ])
    );
}

#[test]
fn semantic_tokens_are_relative_utf16_and_one_line_each() {
    let data = semantic_tokens(MODULE)["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_u64().unwrap() as u32)
        .collect::<Vec<_>>();
    let mut absolute = Vec::new();
    let (mut line, mut character) = (0, 0);
    for token in data.chunks(5) {
        line += token[0];
        character = if token[0] == 0 {
            character + token[1]
        } else {
            token[1]
        };
        absolute.push((
            line,
            character,
            token[2],
            TOKEN_TYPES[token[3] as usize],
            token[4],
        ));
    }
    let find = |line: u32, character: u32| {
        absolute
            .iter()
            .find(|token| token.0 == line && token.1 == character)
            .map(|token| (token.2, token.3, token.4))
    };
    // The comment holds a surrogate pair: 23 UTF-16 units.
    assert_eq!(find(2, 0), Some((23, "comment", 0)));
    assert_eq!(find(4, 0), Some((10, "decorator", 0)));
    assert_eq!(find(5, 8), Some((7, "method", 1)));
    assert_eq!(find(5, 21), Some((1, "parameter", 1)));
    // The query string spans two lines and comes as two tokens.
    let query = at(MODULE, "\"ВЫБРАТЬ");
    assert_eq!(find(query.line, query.character), Some((8, "string", 0)));
    assert_eq!(find(7, 1), Some((5, "string", 0)));
    assert_eq!(find(0, 0).map(|token| token.1), Some("macro"));
    assert!(absolute
        .windows(2)
        .all(|pair| (pair[0].0, pair[0].1) < (pair[1].0, pair[1].1)));
}

#[test]
fn signature_help_names_the_method_and_the_active_parameter() {
    let no_modules = |_: &str| None;
    let help = signature_help(MODULE, after(MODULE, "Сложить(А, "), &no_modules, &[]).unwrap();
    assert_eq!(help["activeParameter"], 1);
    let signature = &help["signatures"][0];
    assert_eq!(signature["label"], "Сложить(Знач А, Б = 1) Экспорт");
    assert_eq!(
        signature["parameters"],
        json!([{"label": [8, 14]}, {"label": [16, 21]}])
    );
    assert_eq!(signature["documentation"]["kind"], "plaintext");
    assert_eq!(
        signature["documentation"]["value"],
        "Складывает 😀 числа.\nВторое необязательно."
    );

    // A common module call resolves through the given text; only an
    // exported method has a signature.
    let common = "Процедура Метод(Первый, Второй) Экспорт\nКонецПроцедуры\nПроцедура Скрытый(А)\nКонецПроцедуры\n";
    let modules = |name: &str| (name == "ОбщМод").then(|| common.to_owned());
    let help = signature_help(MODULE, after(MODULE, "Метод(А, "), &modules, &[]).unwrap();
    assert_eq!(
        help["signatures"][0]["label"],
        "Метод(Первый, Второй) Экспорт"
    );
    assert_eq!(help["activeParameter"], 1);
    let hidden = MODULE.replace("ОбщМод.Метод", "ОбщМод.Скрытый");
    assert!(signature_help(&hidden, after(&hidden, "Скрытый(А, "), &modules, &[]).is_none());
    assert!(signature_help(MODULE, after(MODULE, "Метод(А, "), &no_modules, &[]).is_none());
}

struct Space {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn layout(root: &Path) -> SpaceLayout {
    SpaceLayout {
        folder: WorkspaceFolder {
            name: "УТ".to_owned(),
            root: root.to_owned(),
        },
        source_sets: vec![SourceSetLayout {
            name: "main".to_owned(),
            kind: SourceSetKind::Configuration,
            root: root.join("src"),
            profile: PlatformProfile::v8_3_27(),
        }],
    }
}

fn reader(
    source: &Path,
    documents: Option<unica_workspace::documents::SessionDocuments>,
) -> ProviderReadAuthority {
    let root = Arc::new(unica_workspace::read::open_source_directory(source).unwrap());
    let read = ProviderReadAuthority::new("main", "actor-main", SourceSetKind::Configuration, root);
    match documents {
        Some(documents) => read.with_documents(documents),
        None => read,
    }
}

#[test]
fn signature_help_reads_the_open_buffers_not_the_disk() {
    let temp = tempfile::tempdir().unwrap();
    let space = Space {
        root: temp.path().canonicalize().unwrap(),
        _temp: temp,
    };
    let caller = space.root.join("src/CommonModules/Вызов/Ext/Module.bsl");
    std::fs::create_dir_all(caller.parent().unwrap()).unwrap();
    std::fs::write(&caller, "// на диске вызова нет\n").unwrap();
    let mut documents = LspDocuments::new("editor", vec![layout(&space.root)]);
    let overlay = DocumentOverlay::default();
    let caller_uri = documents.body_uri_of_file(&caller);
    let typed = "Процедура П()\n\tНовыйМодуль.Посчитать(1, \nКонецПроцедуры\n";
    documents
        .open(&overlay, &caller_uri, 1, typed.to_owned())
        .unwrap();
    // The common module exists only as an open buffer.
    let callee_uri = documents
        .spaces()
        .format(&crate::application::lsp::uri::DocumentUri {
            space: "ут".to_owned(),
            address: "main:CommonModule.НовыйМодуль.Body".to_owned(),
        })
        .unwrap();
    documents
        .open(
            &overlay,
            &callee_uri,
            1,
            "Функция Посчитать(Сколько, Чего) Экспорт\nКонецФункции\n".to_owned(),
        )
        .unwrap();

    let read = reader(
        &space.root.join("src"),
        Some(overlay.session_documents("editor", "main").unwrap()),
    );
    let help = documents
        .signature_help(&caller_uri, after(typed, "(1, "), &read, &[])
        .unwrap()
        .unwrap();
    assert_eq!(
        help["signatures"][0]["label"],
        "Посчитать(Сколько, Чего) Экспорт"
    );
    assert_eq!(help["activeParameter"], 1);
    // The qualifier is matched without case, against the buffer's name.
    let lower = typed.replace("НовыйМодуль", "новыймодуль");
    documents
        .change(
            &overlay,
            &caller_uri,
            2,
            &[crate::application::lsp::document::ContentChange::Full(
                lower.clone(),
            )],
        )
        .unwrap();
    let read = reader(
        &space.root.join("src"),
        Some(overlay.session_documents("editor", "main").unwrap()),
    );
    let help = documents
        .signature_help(&caller_uri, after(&lower, "(1, "), &read, &[])
        .unwrap()
        .unwrap();
    assert_eq!(
        help["signatures"][0]["label"],
        "Посчитать(Сколько, Чего) Экспорт"
    );

    // Another connection reads the disk, where neither exists.
    let other = LspDocuments::new("other", vec![layout(&space.root)]);
    let disk = reader(&space.root.join("src"), None);
    assert_eq!(
        other
            .signature_help(&caller_uri, after(typed, "(1, "), &disk, &[])
            .unwrap(),
        None
    );
}

/// Every feature on a large module. Point `UNICA_LSP_UT_SOURCE` at a
/// Designer dump of a large configuration (its `src`), run with
/// `--ignored --nocapture`, best in release.
#[test]
#[ignore = "needs a large local Designer dump in UNICA_LSP_UT_SOURCE"]
fn features_on_a_large_common_module() {
    let Some(source) = std::env::var_os("UNICA_LSP_UT_SOURCE").map(PathBuf::from) else {
        return;
    };
    let text =
        std::fs::read_to_string(source.join("CommonModules/ОбщегоНазначения/Ext/Module.bsl"))
            .unwrap();
    let text = text.trim_start_matches('\u{feff}');
    let measure = |name: &str, run: &dyn Fn() -> usize| {
        let mut best = f64::MAX;
        let mut size = 0;
        for _ in 0..5 {
            let started = Instant::now();
            size = run();
            best = best.min(started.elapsed().as_secs_f64() * 1000.0);
        }
        println!("{name}: {size} items, best of 5 {best:.1} ms");
    };
    println!("{} lines, {} bytes", text.lines().count(), text.len());
    measure("documentSymbol", &|| {
        fn count(symbols: &Value) -> usize {
            symbols
                .as_array()
                .unwrap()
                .iter()
                .map(|symbol| 1 + count(&symbol["children"]))
                .sum()
        }
        count(&document_symbols(text))
    });
    measure("foldingRange", &|| {
        folding_ranges(text).as_array().unwrap().len()
    });
    measure("semanticTokens", &|| {
        semantic_tokens(text)["data"].as_array().unwrap().len() / 5
    });
    // The last call of a method of this module, just inside its `(`.
    let call = unica_bsl_parser::parse(text)
        .editor_symbols()
        .iter()
        .flat_map(|symbol| std::iter::once(symbol).chain(symbol.children.iter()))
        .filter_map(|symbol| {
            text.rfind(&format!("{}(", symbol.name))
                .map(|at| at + symbol.name.len() + 1)
        })
        .max()
        .unwrap();
    let position = LineIndex::new(text).position(text, call);
    measure("signatureHelp", &|| {
        usize::from(signature_help(text, position, &|_| None, &[]).is_some())
    });
}

struct FixtureSources {
    global: Vec<GlobalContextMember>,
    stale: bool,
    form: bool,
}

impl CompletionSources for FixtureSources {
    fn global_context(&self) -> &[GlobalContextMember] {
        &self.global
    }

    fn metadata_names(&self, kind: &str) -> Vec<String> {
        match kind {
            "CommonModule" => vec!["ОбщМод".to_owned()],
            "Catalog" => vec!["Валюты".to_owned(), "Номенклатура".to_owned()],
            _ => Vec::new(),
        }
    }

    fn common_module_text(&self, name: &str) -> Option<String> {
        (name.to_lowercase() == "общмод").then(|| {
            "// Описание метода.\nПроцедура Метод(Первый) Экспорт\nКонецПроцедуры\nПроцедура Скрытый()\nКонецПроцедуры\n"
                .to_owned()
        })
    }

    fn object_members(&self, kind: &str, name: &str) -> Option<ObjectMembers> {
        (kind == "Catalog" && name == "Номенклатура").then(|| ObjectMembers {
            attributes: vec!["Артикул".to_owned()],
            tabular_sections: vec![(
                "Штрихкоды".to_owned(),
                vec!["Штрихкод".to_owned(), "Упаковка".to_owned()],
            )],
        })
    }

    fn module_owner(&self) -> Option<ModuleOwner> {
        Some(ModuleOwner {
            kind: "Catalog".to_owned(),
            name: "Номенклатура".to_owned(),
            form: self.form,
        })
    }

    fn cancelled(&self) -> bool {
        self.stale
    }
}

fn labels(answer: &Value) -> Vec<String> {
    answer["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["label"].as_str().unwrap().to_owned())
        .collect()
}

const OBJECT_MODULE: &str = "Перем МодульнаяПеременная;

// Пересчитывает строки.
Процедура Пересчитать(Режим) Экспорт
\tПерем Сумма;
\tЭлемент = Справочники.Номенклатура.СоздатьЭлемент();
\tДля Каждого Строка Из ЭтотОбъект.Штрихкоды Цикл
\t\tСтрока.
\tКонецЦикла;
\tЭлемент.
\tСправочники.Ном
\tОбщМод.
\tЭтотОбъект.
\tСу // Сумма
\tСу
КонецПроцедуры
";

fn completion_at(text: &str, needle: &str, sources: &FixtureSources) -> Option<Value> {
    completion(text, after(text, needle), sources)
}

#[test]
fn completion_offers_scope_names_metadata_and_object_members() {
    let sources = FixtureSources {
        global: vec![GlobalContextMember {
            kind: GlobalContextMemberKind::Method,
            name_ru: "Сообщить".to_owned(),
            name_en: Some("Message".to_owned()),
            signature_ru: Some("Сообщить(<?>,);".to_owned()),
            signature_en: None,
            parameters: vec!["ТекстСообщения".to_owned(), "Статус".to_owned()],
        }],
        stale: false,
        form: false,
    };
    let at = OBJECT_MODULE.rfind("\tСу\n").unwrap() + "\tСу".len();
    let answer = completion(
        OBJECT_MODULE,
        LineIndex::new(OBJECT_MODULE).position(OBJECT_MODULE, at),
        &sources,
    )
    .unwrap();
    let names = labels(&answer);
    for expected in [
        "Режим",
        "Сумма",
        "Элемент",
        "Строка",
        "МодульнаяПеременная",
        "Пересчитать",
        "Артикул",
        "Штрихкоды",
        "ОбщМод",
        "Справочники",
        "Документы",
        "Сообщить",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected} in {names:?}"
        );
    }
    // Locals sort before module names and the global context.
    let rank = |label: &str| {
        answer["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["label"] == label)
            .unwrap()["sortText"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(rank("Сумма") < rank("Пересчитать") && rank("Пересчитать") < rank("Сообщить"));
    // The edit replaces the typed «Су».
    let item = &answer["items"][0];
    let line = LineIndex::new(OBJECT_MODULE)
        .position(OBJECT_MODULE, at)
        .line;
    assert_eq!(
        item["textEdit"]["range"],
        json!({"start": {"line": line, "character": 1}, "end": {"line": line, "character": 3}})
    );

    let members = |needle: &str| labels(&completion_at(OBJECT_MODULE, needle, &sources).unwrap());
    assert_eq!(members("Справочники.Ном"), ["Валюты", "Номенклатура"]);
    assert_eq!(members("\tОбщМод."), ["Метод"]);
    assert_eq!(members("\tЭтотОбъект."), ["Артикул", "Штрихкоды"]);
    assert_eq!(members("\tЭлемент."), ["Артикул", "Штрихкоды"]);
    assert_eq!(members("\t\tСтрока."), ["Штрихкод", "Упаковка"]);

    // No completion in a comment; a stale request answers nothing.
    assert_eq!(completion_at(OBJECT_MODULE, "// Сум", &sources), None);
    let stale = FixtureSources {
        global: Vec::new(),
        stale: true,
        form: false,
    };
    assert_eq!(completion_at(OBJECT_MODULE, "\tОбщМод.", &stale), None);
}

#[test]
fn resolve_adds_the_method_description() {
    let sources = FixtureSources {
        global: Vec::new(),
        stale: false,
        form: false,
    };
    let answer = completion_at(OBJECT_MODULE, "\tОбщМод.", &sources).unwrap();
    let item = answer["items"][0].clone();
    assert_eq!(
        item["data"],
        json!({"commonModule": "ОбщМод", "method": "Метод"})
    );
    let resolved = resolve_completion(item, &sources.common_module_text("ОбщМод").unwrap());
    assert_eq!(resolved["documentation"]["value"], "Описание метода.");
    assert_eq!(resolved["detail"], "Метод(Первый) Экспорт");
}

#[test]
fn completion_reads_the_buffer_and_the_source_set_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("src");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/acceptance/workspace-catalog-tabular-properties/src");
    std::fs::create_dir_all(source.join("Catalogs")).unwrap();
    std::fs::copy(
        fixture.join("Catalogs/Items.xml"),
        source.join("Catalogs/Items.xml"),
    )
    .unwrap();
    let configuration = std::fs::read_to_string(fixture.join("Configuration.xml"))
        .unwrap()
        .replace(
            "<Catalog>Items</Catalog>",
            "<CommonModule>Утилиты</CommonModule><Catalog>Items</Catalog>",
        );
    std::fs::write(source.join("Configuration.xml"), configuration).unwrap();
    let module = source.join("Catalogs/Items/Ext/ObjectModule.bsl");
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::write(&module, "").unwrap();

    let mut documents = LspDocuments::new("editor", vec![layout(&root)]);
    let overlay = DocumentOverlay::default();
    let uri = documents.body_uri_of_file(&module);
    let typed = "Процедура П()\n\tДля Каждого С Из ЭтотОбъект.Lines Цикл\n\t\tС.\n\tКонецЦикла;\n\tЭтотОбъект.\n\tCatalogs.\n\tУтилиты.\nКонецПроцедуры\n";
    documents.open(&overlay, &uri, 1, typed.to_owned()).unwrap();
    let utilities = documents
        .spaces()
        .format(&crate::application::lsp::uri::DocumentUri {
            space: "ут".to_owned(),
            address: "main:CommonModule.Утилиты.Body".to_owned(),
        })
        .unwrap();
    documents
        .open(
            &overlay,
            &utilities,
            1,
            "Функция Сумма(А, Б) Экспорт\nКонецФункции\n".to_owned(),
        )
        .unwrap();
    let read = reader(
        &source,
        Some(overlay.session_documents("editor", "main").unwrap()),
    );
    let complete = |needle: &str| {
        documents
            .completion(&overlay, &uri, after(typed, needle), &read, &[], &|| false)
            .unwrap()
            .map(|answer| labels(&answer))
    };
    assert_eq!(complete("\t\tС."), Some(vec!["Quantity".to_owned()]));
    assert_eq!(
        complete("\tЭтотОбъект."),
        Some(vec!["Lines".to_owned(), "Notes".to_owned()])
    );
    assert_eq!(complete("\tCatalogs."), Some(vec!["Items".to_owned()]));
    assert_eq!(complete("\tУтилиты."), Some(vec!["Сумма".to_owned()]));

    // An answer computed for a version the buffer moved past is dropped.
    let position = after(typed, "\tУтилиты.");
    let stale = documents
        .completion(&overlay, &uri, position, &read, &[], &|| {
            overlay
                .put(
                    "editor",
                    "main",
                    Path::new("Catalogs/Items/Ext/ObjectModule.bsl"),
                    unica_workspace::documents::DocumentText {
                        version: 2,
                        text: Arc::from(""),
                    },
                )
                .is_err()
        })
        .unwrap();
    assert_eq!(stale, None);
}

/// Completion latency on a large module and configuration. Point
/// `UNICA_LSP_UT_SOURCE` at the dump's `src` and, for the global context,
/// `UNICA_PLATFORM_ROOT` at an installed platform; run in release with
/// `--ignored --nocapture`.
#[test]
#[ignore = "needs a large local Designer dump in UNICA_LSP_UT_SOURCE"]
fn completion_latency_on_a_large_configuration() {
    let Some(source) = std::env::var_os("UNICA_LSP_UT_SOURCE").map(PathBuf::from) else {
        return;
    };
    let source = source.canonicalize().unwrap();
    let global = match std::env::var_os("UNICA_PLATFORM_ROOT") {
        Some(root) => {
            let started = Instant::now();
            let members =
                unica_documentation_sources::platform_help::provider::global_context_members(
                    Path::new(&root),
                )
                .unwrap();
            println!(
                "global context: {} members, indexed in {:.1} s",
                members.len(),
                started.elapsed().as_secs_f64()
            );
            members
        }
        None => Vec::new(),
    };
    let mut documents = LspDocuments::new(
        "editor",
        vec![SpaceLayout {
            folder: WorkspaceFolder {
                name: "УТ".to_owned(),
                root: source.parent().unwrap().to_owned(),
            },
            source_sets: vec![SourceSetLayout {
                name: "main".to_owned(),
                kind: SourceSetKind::Configuration,
                root: source.clone(),
                profile: PlatformProfile::v8_3_27(),
            }],
        }],
    );
    let overlay = DocumentOverlay::default();
    let module = source.join("CommonModules/ОбщегоНазначения/Ext/Module.bsl");
    let uri = documents.body_uri_of_file(&module);
    let text = std::fs::read_to_string(&module).unwrap();
    let text = text.trim_start_matches('\u{feff}').to_owned();
    // Type into the last method: a plain name, then a manager.
    let end = text
        .rfind("КонецФункции")
        .or_else(|| text.rfind("КонецПроцедуры"))
        .unwrap();
    let mut typed = text.clone();
    typed.insert_str(
        end,
        "Спр\n\tСправочники.\n\tОбщегоНазначенияКлиентСервер.\n",
    );
    documents.open(&overlay, &uri, 1, typed.clone()).unwrap();
    let read = reader(
        &source,
        Some(overlay.session_documents("editor", "main").unwrap()),
    );
    let lines = LineIndex::new(&typed);
    for (name, at) in [
        ("plain name", end + "Спр".len()),
        ("Справочники.", end + "Спр\n\tСправочники.".len()),
        (
            "common module",
            end + "Спр\n\tСправочники.\n\tОбщегоНазначенияКлиентСервер.".len(),
        ),
    ] {
        let position = lines.position(&typed, at);
        let mut times = Vec::new();
        let mut size = 0;
        for _ in 0..5 {
            let started = Instant::now();
            let answer = documents
                .completion(&overlay, &uri, position, &read, &global, &|| false)
                .unwrap()
                .unwrap();
            times.push(started.elapsed().as_secs_f64() * 1000.0);
            size = answer["items"].as_array().unwrap().len();
        }
        println!(
            "completion {name}: {size} items, first {:.1} ms, best of 5 {:.1} ms",
            times[0],
            times.iter().cloned().fold(f64::MAX, f64::min)
        );
    }
}

#[test]
fn the_module_object_follows_the_module_kind_and_yields_to_variables() {
    let object = FixtureSources {
        global: Vec::new(),
        stale: false,
        form: false,
    };
    let form = FixtureSources {
        form: true,
        ..FixtureSources {
            global: Vec::new(),
            stale: false,
            form: false,
        }
    };
    let text = "Процедура П()\n\tЭтотОбъект.\n\tОбъект.\n\tСт\nКонецПроцедуры\n";
    let members = |needle: &str, sources: &FixtureSources| {
        labels(&completion_at(text, needle, sources).unwrap())
    };
    assert_eq!(members("\tЭтотОбъект.", &object), ["Артикул", "Штрихкоды"]);
    assert!(members("\tОбъект.", &object).is_empty());
    // In a form `ЭтотОбъект` is the form; the object is `Объект`, and its
    // attributes are not bare names of the module.
    assert!(members("\tЭтотОбъект.", &form).is_empty());
    assert_eq!(members("\tОбъект.", &form), ["Артикул", "Штрихкоды"]);
    assert!(!members("\tСт", &form).contains(&"Артикул".to_owned()));
    assert!(members("\tСт", &object).contains(&"Артикул".to_owned()));

    // A variable named `Объект` takes its own type, even in a form; the
    // metadata name matches its registration in any case.
    let assigned = "Процедура П()\n\tОбъект = справочники.номенклатура.СоздатьЭлемент();\n\tОбъект.\nКонецПроцедуры\n";
    let other_owner = FixtureSources {
        form: true,
        ..FixtureSources {
            global: Vec::new(),
            stale: false,
            form: false,
        }
    };
    assert_eq!(
        labels(&completion_at(assigned, "\tОбъект.", &other_owner).unwrap()),
        ["Артикул", "Штрихкоды"]
    );
    let reassigned = "Процедура П()\n\tОбъект = 1;\n\tОбъект.\nКонецПроцедуры\n";
    assert!(labels(&completion_at(reassigned, "\tОбъект.", &form).unwrap()).is_empty());
}

#[test]
fn platform_methods_are_signed_from_the_global_context() {
    let global = [GlobalContextMember {
        kind: GlobalContextMemberKind::Method,
        name_ru: "ЗначениеЗаполнено".to_owned(),
        name_en: Some("ValueIsFilled".to_owned()),
        signature_ru: None,
        signature_en: None,
        parameters: vec!["Значение".to_owned()],
    }];
    let text = "Процедура П()\n\tЕсли значениезаполнено( Тогда\n\tValueIsFilled(\nКонецПроцедуры\n";
    for needle in ["значениезаполнено(", "ValueIsFilled("] {
        let help = signature_help(text, after(text, needle), &|_| None, &global).unwrap();
        assert_eq!(
            help["signatures"][0]["label"],
            "ЗначениеЗаполнено(Значение)"
        );
        assert_eq!(
            help["signatures"][0]["parameters"],
            json!([{"label": [18, 26]}])
        );
        assert_eq!(help["activeParameter"], 0);
    }
    // A method of the module shadows the platform one.
    let shadowed = "Функция ЗначениеЗаполнено(А, Б)\nКонецФункции\nПроцедура П()\n\tЗначениеЗаполнено(\nКонецПроцедуры\n";
    let help = signature_help(
        shadowed,
        after(shadowed, "\tЗначениеЗаполнено("),
        &|_| None,
        &global,
    )
    .unwrap();
    assert_eq!(help["signatures"][0]["label"], "ЗначениеЗаполнено(А, Б)");
}
