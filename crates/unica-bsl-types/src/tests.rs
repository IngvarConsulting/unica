use super::*;

fn page(body: String) -> String {
    format!("<html><body>{body}</body></html>")
}

const QUERY_RESULT: &str =
    "<a href=\"v8help://SyntaxHelperContext/objects/c/Result.html\">Результат</a>";

fn russian() -> Vec<(String, String)> {
    vec![
        ("objects/c/Sample.html".into(), page("<p class=\"V8SH_title\">Пример (Sample)</p><p class=\"V8SH_chapter\">Доступность:</p><p>Сервер.</p>".into())),
        ("objects/c/Result.html".into(), page("<p class=\"V8SH_title\">Результат (Result)</p>".into())),
        ("objects/c/Sample/methods/Run1.html".into(), page(format!(
            "<p class=\"V8SH_heading\">Выполнить (Run)</p><p class=\"V8SH_chapter\">Синтаксис:</p>Выполнить(&lt;Режим&gt;)\
             <p class=\"V8SH_chapter\">Параметры:</p><div class=\"V8SH_rubric\"> <p>&lt;Режим&gt; (необязательный)</div>Тип: <a href=\"v8help://SyntaxHelperContext/objects/gone/Mode.html\">Режим</a>.\
             <p class=\"V8SH_chapter\">Возвращаемое значение:</p>Тип: {QUERY_RESULT}."))),
        ("objects/c/Sample/ctors/ctor1.html".into(), page(
            "<p class=\"V8SH_heading\">По тексту</p><p class=\"V8SH_chapter\">Синтаксис:</p>Новый Пример(&lt;Текст&gt;)\
             <p class=\"V8SH_chapter\">Параметры:</p><div class=\"V8SH_rubric\"> <p>&lt;Текст&gt; (обязательный)</div>Тип: <a href=\"v8help://SyntaxHelperLanguage/def_String\">Строка</a>.".into())),
        ("objects/c/Sample/properties/Text1.html".into(), page(
            "<p class=\"V8SH_heading\">Текст (Text)</p><p class=\"V8SH_chapter\">Использование:</p>Чтение и запись.".into())),
        ("objects/Global context.html".into(), page("<p class=\"V8SH_title\">Глобальный контекст (Global context)</p>".into())),
        ("objects/Global context/methods/Filled1.html".into(), page(
            "<p class=\"V8SH_heading\">ЗначениеЗаполнено (ValueIsFilled)</p><p class=\"V8SH_chapter\">Синтаксис:</p>ЗначениеЗаполнено(&lt;Значение&gt;)\
             <p class=\"V8SH_chapter\">Параметры:</p><div class=\"V8SH_rubric\"> <p>&lt;Значение&gt; (обязательный)</div>Тип: Произвольный.\
             <p class=\"V8SH_chapter\">Возвращаемое значение:</p>Тип: <a href=\"v8help://SyntaxHelperLanguage/def_Boolean\">Булево</a>.".into())),
    ]
}

fn english() -> Vec<(String, String)> {
    vec![
        ("objects/c/Sample/methods/Run1.html".into(), page(
            "<p class=\"V8SH_heading\">Run</p><p class=\"V8SH_chapter\">Syntax:</p>Run(&lt;Mode&gt;)\
             <p class=\"V8SH_chapter\">Parameters:</p><div class=\"V8SH_rubric\"> <p>&lt;Mode&gt; (optional)</div>Type: x.".into())),
        ("objects/c/Sample/ctors/ctor1.html".into(), page(
            "<p class=\"V8SH_heading\">Based on text</p><p class=\"V8SH_chapter\">Syntax:</p>New Sample(&lt;Text&gt;)\
             <p class=\"V8SH_chapter\">Parameters:</p><div class=\"V8SH_rubric\"> <p>&lt;Text&gt; (required)</div>Type: x.".into())),
    ]
}

#[test]
fn one_record_per_type_and_member_in_both_languages_with_types_by_page() {
    let catalog = build_catalog("8.3.27.0", &russian(), &english());
    assert_eq!(catalog.generator, GENERATOR);
    assert_eq!(catalog.types.len(), 3);
    let sample = catalog.type_by_name("sample").unwrap();
    assert_eq!(
        (sample.ru.as_str(), sample.en.as_str()),
        ("Пример", "Sample")
    );
    assert_eq!(sample.contexts, vec![Context::Server]);

    let run = sample.method("выполнить").unwrap();
    assert_eq!(run.en, "Run");
    let variant = &run.variants[0];
    assert_eq!(
        (variant.params[0].ru.as_str(), variant.params[0].en.as_str()),
        ("Режим", "Mode")
    );
    assert!(variant.params[0].optional);
    // A link to a page the help does not have stays as written.
    assert_eq!(
        variant.params[0].types[0].ty,
        TypeName::Unresolved("objects/gone/Mode".into())
    );
    assert_eq!(
        catalog
            .type_by_id(match &variant.returns[0].ty {
                TypeName::Context(id) => id,
                other => panic!("{other:?}"),
            })
            .unwrap()
            .en,
        "Result"
    );

    assert_eq!(sample.constructors[0].name, "По тексту");
    assert_eq!(sample.constructors[0].params[0].en, "Text");
    let text = sample.property("TEXT").unwrap();
    assert!(!text.read_only);
    assert!(
        text.types.is_empty(),
        "no type in the help is no type, not a guess"
    );

    let global = catalog.type_by_name("Глобальный контекст").unwrap();
    let filled = global.method("ValueIsFilled").unwrap();
    assert_eq!(
        filled.variants[0].returns[0].ty,
        TypeName::Primitive("Boolean".into())
    );
    assert_eq!(filled.variants[0].params[0].en, "");
}

/// The catalog of a real installation. Point `UNICA_PLATFORM_ROOT` at an
/// installed platform version and run with `--ignored --nocapture`.
#[test]
#[ignore = "needs an installed platform in UNICA_PLATFORM_ROOT"]
fn the_catalog_of_an_installation() {
    let Some(root) = std::env::var_os("UNICA_PLATFORM_ROOT").map(std::path::PathBuf::from) else {
        return;
    };
    let platform = root.file_name().unwrap().to_string_lossy().into_owned();
    let containers = Containers::of_installation(&root);
    let state = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let built = load_or_build(&platform, &containers, state.path()).unwrap();
    let cold = started.elapsed();
    let started = std::time::Instant::now();
    let cached = load_or_build(&platform, &containers, state.path()).unwrap();
    let warm = started.elapsed();
    assert_eq!(built, cached);
    let size = std::fs::metadata(catalog_path(state.path(), &platform))
        .unwrap()
        .len();

    let methods = built
        .types
        .iter()
        .flat_map(|ty| &ty.methods)
        .collect::<Vec<_>>();
    let properties = built
        .types
        .iter()
        .flat_map(|ty| &ty.properties)
        .collect::<Vec<_>>();
    let typed_returns = methods
        .iter()
        .filter(|method| {
            method
                .variants
                .iter()
                .any(|variant| !variant.returns.is_empty())
        })
        .count();
    let typed_properties = properties
        .iter()
        .filter(|property| !property.types.is_empty())
        .count();
    let with_contexts = methods
        .iter()
        .filter(|method| !method.contexts.is_empty())
        .count();
    let mut unresolved = std::collections::BTreeSet::new();
    let mut other_contexts = std::collections::BTreeSet::new();
    for ty in &built.types {
        for method in &ty.methods {
            other_contexts.extend(
                method
                    .contexts
                    .iter()
                    .filter(|c| matches!(c, Context::Other(_)))
                    .cloned(),
            );
            for variant in &method.variants {
                for reference in variant
                    .returns
                    .iter()
                    .chain(variant.params.iter().flat_map(|p| &p.types))
                {
                    if let TypeName::Unresolved(name) = &reference.ty {
                        unresolved.insert(name.clone());
                    }
                }
            }
        }
    }
    let arrays = methods
        .iter()
        .flat_map(|method| &method.variants)
        .flat_map(|variant| &variant.returns)
        .filter(|reference| reference.element.is_some())
        .count();
    println!(
        "{platform}: {} types, {} methods ({typed_returns} with a return type, {with_contexts} with contexts), \
         {} properties ({typed_properties} typed), {} events, {} constructors, {arrays} arrays with an element; \
         built {:.1} s, from cache {:.0} ms, {:.1} MB on disk; {} unresolved type names, other contexts {:?}",
        built.types.len(),
        methods.len(),
        properties.len(),
        built.types.iter().map(|ty| ty.events.len()).sum::<usize>(),
        built.types.iter().map(|ty| ty.constructors.len()).sum::<usize>(),
        cold.as_secs_f64(),
        warm.as_secs_f64() * 1000.0,
        size as f64 / 1e6,
        unresolved.len(),
        other_contexts,
    );
    println!(
        "unresolved sample: {:?}",
        unresolved.iter().take(8).collect::<Vec<_>>()
    );

    let returns = |ty: &str, method: &str| {
        let method = built.type_by_name(ty).unwrap().method(method).unwrap();
        method.variants[0]
            .returns
            .iter()
            .map(|reference| match &reference.ty {
                TypeName::Context(id) => built.type_by_id(id).unwrap().ru.clone(),
                TypeName::Primitive(name) => name.clone(),
                TypeName::Unresolved(name) => format!("?{name}"),
            })
            .collect::<Vec<_>>()
    };
    // The help gives both: a batch query may end without a result.
    assert_eq!(
        returns("Запрос", "Выполнить"),
        ["РезультатЗапроса", "Undefined"]
    );
    assert_eq!(
        returns("РезультатЗапроса", "Выбрать"),
        ["ВыборкаИзРезультатаЗапроса"]
    );
    assert_eq!(
        returns("Глобальный контекст", "ЗначениеЗаполнено"),
        ["Boolean"]
    );
    let template = built
        .type_by_name("Глобальный контекст")
        .unwrap()
        .method("StrTemplate")
        .unwrap();
    println!(
        "СтрШаблон: {:?}",
        template.variants[0]
            .params
            .iter()
            .map(|p| (&p.ru, &p.en, p.optional))
            .collect::<Vec<_>>()
    );
    assert_eq!(template.variants[0].params[0].ru, "Шаблон");
    assert_eq!(template.variants[0].params[0].en, "Template");
}
