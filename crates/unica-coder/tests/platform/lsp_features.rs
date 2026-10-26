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
    let help = signature_help(MODULE, after(MODULE, "Сложить(А, "), &no_modules).unwrap();
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
    let help = signature_help(MODULE, after(MODULE, "Метод(А, "), &modules).unwrap();
    assert_eq!(
        help["signatures"][0]["label"],
        "Метод(Первый, Второй) Экспорт"
    );
    assert_eq!(help["activeParameter"], 1);
    let hidden = MODULE.replace("ОбщМод.Метод", "ОбщМод.Скрытый");
    assert!(signature_help(&hidden, after(&hidden, "Скрытый(А, "), &modules).is_none());
    assert!(signature_help(MODULE, after(MODULE, "Метод(А, "), &no_modules).is_none());
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
    let file_uri = |path: &Path| String::from(url::Url::from_file_path(path).unwrap());
    let caller_uri = documents.uri_of_file(&file_uri(&caller)).unwrap();
    let typed = "Процедура П()\n\tНовыйМодуль.Посчитать(1, \nКонецПроцедуры\n";
    documents
        .open(&overlay, &caller_uri, 1, typed.to_owned())
        .unwrap();
    // The common module exists only as an open buffer.
    let callee_uri = documents
        .spaces()
        .format(&crate::application::lsp::uri::DocumentUri {
            space: "ут".to_owned(),
            address: "main:CommonModule.НовыйМодуль".to_owned(),
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
        .signature_help(&caller_uri, after(typed, "(1, "), &read)
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
        .signature_help(&caller_uri, after(&lower, "(1, "), &read)
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
            .signature_help(&caller_uri, after(typed, "(1, "), &disk)
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
        usize::from(signature_help(text, position, &|_| None).is_some())
    });
}
