//! Каталог типов BSL, построенный из справки реальной установки через
//! `PlatformHelpPages`. Материалы вендора в репозиторий не попадают,
//! поэтому проверка помечена `#[ignore]`.

use super::corpus::PlatformHelpPages;
use unica_bsl_types::{catalog_path, load_or_build, Containers, Context, TypeName};

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
    let built = load_or_build(&platform, &containers, state.path(), &PlatformHelpPages).unwrap();
    let cold = started.elapsed();
    let started = std::time::Instant::now();
    let cached = load_or_build(&platform, &containers, state.path(), &PlatformHelpPages).unwrap();
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
                TypeName::Applied(applied) => format!("!{}.{}", applied.kind, applied.name),
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
