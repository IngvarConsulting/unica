use super::*;
use crate::infrastructure::lsp_features::{
    completion, signature_help, CompletionSources, ObjectMembers,
};
use unica_bsl_types::{
    AppliedMember, AppliedMemberKind, Method, Param, PlatformType, Property, Signature, TypeRef,
};
use unica_documentation_sources::platform_help::provider::GlobalContextMember;

fn reference(id: &str) -> TypeRef {
    TypeRef {
        ty: TypeName::Context(id.into()),
        element: None,
    }
}

fn method(ru: &str, params: &[&str], returns: &[&str]) -> Method {
    Method {
        ru: ru.into(),
        en: String::new(),
        variants: vec![Signature {
            name: String::new(),
            params: params
                .iter()
                .map(|name| Param {
                    ru: (*name).into(),
                    en: String::new(),
                    optional: false,
                    types: Vec::new(),
                    default: None,
                })
                .collect(),
            returns: returns.iter().map(|id| reference(id)).collect(),
        }],
        contexts: Vec::new(),
        server_call: false,
    }
}

fn ty(
    id: &str,
    ru: &str,
    en: &str,
    properties: Vec<Property>,
    methods: Vec<Method>,
) -> PlatformType {
    PlatformType {
        id: id.into(),
        ru: ru.into(),
        en: en.into(),
        properties,
        methods,
        events: Vec::new(),
        constructors: Vec::new(),
        collection_element: Vec::new(),
        contexts: Vec::new(),
    }
}

fn property(ru: &str, types: &[&str]) -> Property {
    Property {
        ru: ru.into(),
        en: String::new(),
        types: types.iter().map(|id| reference(id)).collect(),
        read_only: true,
        contexts: Vec::new(),
    }
}

fn catalog() -> PlatformCatalog {
    let mut types = vec![
        ty(
            "objects/Global context",
            "Глобальный контекст",
            "Global context",
            vec![property("Справочники", &["cm"])],
            vec![method("ЗначениеЗаполнено", &["Значение"], &[])],
        ),
        ty(
            "cm",
            "СправочникиМенеджер",
            "CatalogsManager",
            vec![property("<Имя справочника>", &["m"])],
            Vec::new(),
        ),
        ty(
            "m",
            "СправочникМенеджер.<Имя справочника>",
            "CatalogManager.<Catalog name>",
            Vec::new(),
            vec![
                method("СоздатьЭлемент", &[], &["o"]),
                method("НайтиПоКоду", &["Код"], &["o"]),
            ],
        ),
        ty(
            "o",
            "СправочникОбъект.<Имя справочника>",
            "CatalogObject.<Catalog name>",
            Vec::new(),
            vec![method("Записать", &[], &[])],
        ),
        ty(
            "q",
            "Запрос",
            "Query",
            Vec::new(),
            vec![method("Выполнить", &[], &["r"])],
        ),
        ty(
            "r",
            "РезультатЗапроса",
            "QueryResult",
            Vec::new(),
            vec![method("Выбрать", &["ТипОбхода", "Группировки"], &["s"])],
        ),
        ty(
            "s",
            "ВыборкаИзРезультатаЗапроса",
            "QueryResultSelection",
            Vec::new(),
            vec![method("Следующий", &[], &[])],
        ),
    ];
    types.sort_by(|a, b| a.id.cmp(&b.id));
    PlatformCatalog {
        platform: "test".into(),
        generator: unica_bsl_types::GENERATOR,
        types,
    }
}

struct Configuration {
    catalog: PlatformCatalog,
    owner: Option<ModuleOwner>,
}

impl AppliedTypes for Configuration {
    fn names(&self, kind: &str) -> Vec<String> {
        if kind == "Catalog" {
            vec!["Валюты".into()]
        } else {
            Vec::new()
        }
    }

    fn members(&self, kind: &str, name: &str, facet: &str) -> Vec<AppliedMember> {
        match (kind, name, facet) {
            ("Catalog", "Валюты", "Object") => vec![AppliedMember {
                name: "Курс".into(),
                kind: AppliedMemberKind::Attribute,
                types: Vec::new(),
                signature: None,
            }],
            _ => Vec::new(),
        }
    }
}

impl CompletionSources for Configuration {
    fn global_context(&self) -> &[GlobalContextMember] {
        &[]
    }

    fn metadata_names(&self, kind: &str) -> Vec<String> {
        AppliedTypes::names(self, kind)
    }

    fn common_module_text(&self, _: &str) -> Option<String> {
        None
    }

    fn object_members(&self, _: &str, _: &str) -> Option<ObjectMembers> {
        None
    }

    fn module_owner(&self) -> Option<ModuleOwner> {
        self.owner.clone()
    }

    fn cancelled(&self) -> bool {
        false
    }

    fn types(&self) -> Option<TypeSources<'_>> {
        Some(TypeSources {
            catalog: &self.catalog,
            applied: self,
            owner: self.owner.clone(),
        })
    }
}

fn configuration() -> Configuration {
    Configuration {
        catalog: catalog(),
        owner: None,
    }
}

/// One module per case: an unfinished line runs into the next statement
/// when parsed, as it does in an editor.
fn module(lines: &str) -> String {
    format!("Процедура П()\n\tЗапрос = Новый Запрос(\"ВЫБРАТЬ 1\");\n\tРезультат = Запрос.Выполнить();\n\tЭлемент = Справочники.Валюты.СоздатьЭлемент();\n{lines}\nКонецПроцедуры\n")
}

fn after(text: &str, needle: &str) -> Utf16Position {
    LineIndex::new(text).position(text, text.find(needle).unwrap() + needle.len())
}

fn labels(answer: &Value) -> Vec<String> {
    answer["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["label"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn completion_after_a_chain_offers_the_members_of_its_type() {
    let configuration = configuration();
    let members = |line: &str| {
        let text = module(line);
        labels(&completion(&text, after(&text, line), &configuration).unwrap())
    };
    // A variable typed by its assignment from a call on a `Новый` value.
    assert_eq!(members("\tРезультат."), ["Выбрать"]);
    // A placeholder of the platform stands for the configuration's objects,
    // and the object a manager creates keeps its applied name.
    assert_eq!(
        members("\tСправочники.Валюты."),
        ["СоздатьЭлемент", "НайтиПоКоду"]
    );
    assert_eq!(members("\tЭлемент."), ["Курс", "Записать"]);
    assert_eq!(members("\tЗапрос.Выполнить().Выбрать()."), ["Следующий"]);
}

#[test]
fn signature_help_and_hover_follow_the_chain() {
    let configuration = configuration();
    let types = configuration.types().unwrap();
    let text = module("\tЗапрос.Выполнить().Выбрать(1, );");
    let help = signature_help(
        &text,
        after(&text, "Выбрать(1, "),
        &|_| None,
        Some(&[]),
        Some(&types),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        help["signatures"][0]["label"],
        "Выбрать(ТипОбхода, Группировки)"
    );
    assert_eq!(
        help["signatures"][0]["parameters"],
        json!([{"label": [8, 17]}, {"label": [19, 30]}])
    );
    assert_eq!(help["activeParameter"], 1);
    assert_eq!(
        help["signatures"][0]["documentation"]["value"],
        "Возвращает: ВыборкаИзРезультатаЗапроса"
    );

    let hover_at = |line: &str, needle: &str| {
        let text = module(line);
        // The last place the needle stands: the line under test.
        let at = LineIndex::new(&text).position(&text, text.rfind(needle).unwrap() + needle.len());
        hover(&text, at, &types).map(|answer| answer["contents"]["value"].clone())
    };
    assert_eq!(
        hover_at("\tРезультат.Выбрать();", "\tРезульта"),
        Some(json!("Тип: РезультатЗапроса"))
    );
    assert_eq!(
        hover_at("\tЭлемент.Записать();", "\tЭлеме"),
        Some(json!("Тип: СправочникОбъект.Валюты"))
    );
    assert_eq!(
        hover_at("\tЗапрос.Выполнить();", "\tЗапрос.Выпол"),
        Some(json!("```bsl\nВыполнить(): РезультатЗапроса\n```"))
    );
    // A name of unknown type has no hover.
    assert_eq!(hover_at("\tНеизвестно = 1;", "Процед"), None);
}

#[test]
fn this_object_is_the_module_owner_of_its_template_type() {
    let configuration = Configuration {
        catalog: catalog(),
        owner: Some(ModuleOwner {
            kind: "Catalog".into(),
            name: "Валюты".into(),
            form: false,
        }),
    };
    let text = "Процедура П()\n\tЭтотОбъект.\nКонецПроцедуры\n";
    assert_eq!(
        labels(&completion(text, after(text, "ЭтотОбъект."), &configuration).unwrap()),
        ["Курс", "Записать"]
    );
}

/// The designer's goal on a real configuration and platform: members after
/// `Запрос.Выполнить().` and `Справочники.Валюты.`. Point
/// `UNICA_LSP_UT_SOURCE` at a Designer dump (`src`) and `UNICA_PLATFORM_ROOT`
/// at an installed platform; run in release with `--ignored --nocapture`.
#[test]
#[ignore = "needs a large local Designer dump and an installed platform"]
fn typed_completion_on_a_large_configuration() {
    use crate::application::lsp::uri::{DocumentUri, WorkspaceFolder};
    use crate::infrastructure::lsp_documents::{LspDocuments, SourceSetLayout, SpaceLayout};
    use crate::infrastructure::v13_read_port::ProviderReadAuthority;
    use std::path::{Path, PathBuf};
    use std::time::Instant;
    use unica_source_contract::SourceSetKind;
    use unica_workspace::documents::DocumentOverlay;

    let (Some(source), Some(root)) = (
        std::env::var_os("UNICA_LSP_UT_SOURCE").map(PathBuf::from),
        std::env::var_os("UNICA_PLATFORM_ROOT").map(PathBuf::from),
    ) else {
        return;
    };
    let source = source.canonicalize().unwrap();
    let state = tempfile::tempdir().unwrap();
    let platform = root.file_name().unwrap().to_string_lossy().into_owned();
    let started = Instant::now();
    let catalog = unica_bsl_types::load_or_build(
        &platform,
        &unica_bsl_types::Containers::of_installation(&root),
        state.path(),
    )
    .unwrap();
    println!(
        "catalog {platform}: {:.1} s",
        started.elapsed().as_secs_f64()
    );
    let mut documents = LspDocuments::new(
        "editor",
        vec![SpaceLayout {
            folder: WorkspaceFolder {
                name: "УТ".into(),
                root: source.parent().unwrap().to_owned(),
            },
            source_sets: vec![SourceSetLayout {
                name: "main".into(),
                kind: SourceSetKind::Configuration,
                root: source.clone(),
                profile: unica_bsl::profile::PlatformProfile::v8_3_27(),
            }],
        }],
    );
    let overlay = DocumentOverlay::default();
    let uri = documents
        .spaces()
        .format(&DocumentUri {
            space: "ут".into(),
            address: "main:CommonModule.ОбщегоНазначения.Body".into(),
        })
        .unwrap();
    let text =
        std::fs::read_to_string(source.join("CommonModules/ОбщегоНазначения/Ext/Module.bsl"))
            .unwrap();
    let text = text.trim_start_matches('\u{feff}');
    let end = text
        .rfind("КонецФункции")
        .or_else(|| text.rfind("КонецПроцедуры"))
        .unwrap();
    for (name, typed, line) in [
        (
            "Запрос.Выполнить().",
            "\tЗапрос = Новый Запрос;\n\tЗапрос.Выполнить().\n",
            "\tЗапрос.Выполнить().",
        ),
        (
            "Справочники.Валюты.",
            "\tСправочники.Валюты.\n",
            "\tСправочники.Валюты.",
        ),
        (
            "Справочники.Валюты.СоздатьЭлемент().",
            "\tСправочники.Валюты.СоздатьЭлемент().\n",
            "\tСправочники.Валюты.СоздатьЭлемент().",
        ),
    ] {
        let mut buffer = text.to_owned();
        buffer.insert_str(end, typed);
        documents.close(&overlay, &uri).ok();
        documents.open(&overlay, &uri, 1, buffer.clone()).unwrap();
        let read = ProviderReadAuthority::new(
            "main",
            "actor-main",
            SourceSetKind::Configuration,
            std::sync::Arc::new(
                unica_workspace::read::open_source_directory(Path::new(&source)).unwrap(),
            ),
        )
        .with_documents(overlay.session_documents("editor", "main").unwrap());
        let at =
            LineIndex::new(&buffer).position(&buffer, end + typed.find(line).unwrap() + line.len());
        let started = Instant::now();
        let answer = documents
            .completion(&overlay, &uri, at, &read, &[], Some(&catalog), &|| false)
            .unwrap()
            .unwrap();
        let elapsed = started.elapsed();
        let labels = labels(&answer);
        println!(
            "{name}: {} items in {:.1} ms; {:?}",
            labels.len(),
            elapsed.as_secs_f64() * 1000.0,
            labels.iter().take(12).collect::<Vec<_>>()
        );
        assert!(!labels.is_empty(), "{name}");
    }
}
