use super::*;
use std::sync::Mutex;

const MODULE_AT: &str = "main:Catalog.Валюты.Module.Manager";
const MODULE_PATH: &str = "Catalogs/Валюты/Ext/ManagerModule.bsl";
const COMMON_AT: &str = "main:CommonModule.Общий";
const COMMON_PATH: &str = "CommonModules/Общий/Ext/Module.bsl";
const COMMON_TEXT: &str = "Процедура Сделать(Значение) Экспорт\nКонецПроцедуры\n";

const TEXT: &str = "Процедура Вызов(Параметр) Экспорт\n\tРезультат = Ф(Параметр);\n\tСсылка = Справочники.Валюты.ПустаяСсылка();\n\tОбщий.Сделать(Ссылка);\n\tВнешний.Метод(Ссылка.Код);\nКонецПроцедуры\n\nФункция Ф(А)\n\tВозврат А;\nКонецФункции\n";

struct Environment;

impl SymbolEnvironment for Environment {
    fn module_address(&self, path: &str) -> Option<String> {
        match path {
            MODULE_PATH => Some(MODULE_AT.to_string()),
            COMMON_PATH => Some(COMMON_AT.to_string()),
            "CommonModules/Внешний/Ext/Module.bsl" => {
                Some("main:CommonModule.Внешний".to_string())
            }
            _ => None,
        }
    }

    fn metadata_address(&self, kind: &str, object: &str, member: Option<&str>) -> Option<String> {
        match (kind, object, member) {
            ("Catalog", "Валюты", None) => Some("main:Catalog.Валюты".to_string()),
            ("Catalog", "Валюты", Some("Код")) => {
                Some("main:Catalog.Валюты.StandardAttribute.Код".to_string())
            }
            ("CommonModule", "Общий", None) => Some(COMMON_AT.to_string()),
            _ => None,
        }
    }

    fn module_text(&self, at: &str) -> Option<String> {
        (at == COMMON_AT).then(|| COMMON_TEXT.to_string())
    }
}

/// A provider that is not `bsl-analyzer`: the service must accept it as is.
#[derive(Default)]
struct StubProvider {
    queries: Mutex<Vec<SymbolQuery>>,
}

fn range(line: usize, start: usize, end: usize) -> SymbolRange {
    SymbolRange {
        start: SymbolPosition {
            line,
            column: start,
        },
        end: SymbolPosition { line, column: end },
    }
}

impl SymbolProvider for StubProvider {
    fn symbol(
        &self,
        query: &SymbolQuery,
        _context: &SymbolRequestContext<'_>,
    ) -> Result<Option<ProviderSymbol>, String> {
        self.queries.lock().unwrap().push(query.clone());
        Ok(Some(ProviderSymbol {
            name: "Метод".to_string(),
            kind: SymbolKind::Method,
            signature: Some("Процедура Метод(Значение) Экспорт".to_string()),
            return_type: None,
            definitions: vec![
                ProviderPlace::Code {
                    path: "CommonModules/Внешний/Ext/Module.bsl".to_string(),
                    range: range(4, 10, 15),
                },
                ProviderPlace::Code {
                    path: "Outside/Unknown.bsl".to_string(),
                    range: range(0, 0, 1),
                },
            ],
        }))
    }

    fn references(
        &self,
        query: &SymbolQuery,
        _context: &SymbolRequestContext<'_>,
    ) -> Result<ProviderPlaces, String> {
        self.queries.lock().unwrap().push(query.clone());
        Ok(ProviderPlaces {
            places: vec![
                ProviderPlace::Code {
                    path: COMMON_PATH.to_string(),
                    range: range(7, 1, 2),
                },
                // The module's own place: its text answers for it.
                ProviderPlace::Code {
                    path: MODULE_PATH.to_string(),
                    range: range(1, 13, 14),
                },
            ],
            complete: true,
        })
    }

    fn calls(
        &self,
        query: &SymbolQuery,
        _direction: CallDirection,
        _context: &SymbolRequestContext<'_>,
    ) -> Result<ProviderPlaces, String> {
        self.queries.lock().unwrap().push(query.clone());
        Ok(ProviderPlaces {
            places: vec![ProviderPlace::Code {
                path: COMMON_PATH.to_string(),
                range: range(0, 10, 17),
            }],
            complete: false,
        })
    }
}

fn document(text: &str, saved: bool) -> SymbolDocument<'_> {
    SymbolDocument {
        at: MODULE_AT,
        path: MODULE_PATH,
        text,
        saved,
    }
}

fn position_of(text: &str, needle: &str, shift: usize) -> SymbolPosition {
    let start = text.find(needle).unwrap_or_else(|| panic!("{needle}"));
    let offset = start
        + text[start..]
            .chars()
            .take(shift)
            .map(char::len_utf8)
            .sum::<usize>();
    let (line, column) = LineIndex::new(text).position(offset);
    SymbolPosition { line, column }
}

fn with_context<T>(run: impl FnOnce(&SymbolRequestContext<'_>) -> T) -> T {
    let cancellation = CancellationToken::new();
    run(&SymbolRequestContext {
        deadline: ProviderDeadline::no_deadline(),
        cancellation: &cancellation,
    })
}

#[test]
fn module_names_are_answered_from_its_own_text_without_a_provider() {
    let environment = Environment;
    let service = SymbolService::new(None, &environment);
    with_context(|context| {
        let call = position_of(TEXT, "Ф(Параметр)", 0);
        let definition = service.definition(&document(TEXT, true), call, context);
        assert!(definition.complete, "{definition:?}");
        assert_eq!(
            definition.value,
            [SymbolLocation {
                at: MODULE_AT.to_string(),
                range: Some(range(7, 8, 9)),
            }]
        );

        let parameter = position_of(TEXT, "Параметр);", 0);
        let references = service.references(&document(TEXT, true), parameter, context);
        assert!(references.complete);
        assert_eq!(references.value.len(), 2);

        let hover = service.hover(&document(TEXT, true), parameter, context);
        assert_eq!(hover.value.as_ref().unwrap().kind, SymbolKind::Parameter);
    });
}

#[test]
fn metadata_and_common_module_names_resolve_to_unica_nodes() {
    let environment = Environment;
    let provider = StubProvider::default();
    let service = SymbolService::new(Some(&provider), &environment);
    with_context(|context| {
        let catalog = service.definition(
            &document(TEXT, true),
            position_of(TEXT, "Валюты.ПустаяСсылка", 1),
            context,
        );
        assert!(catalog.complete);
        assert_eq!(
            catalog.value,
            [SymbolLocation {
                at: "main:Catalog.Валюты".to_string(),
                range: None,
            }]
        );

        let common = service.definition(
            &document(TEXT, false),
            position_of(TEXT, "Сделать(Ссылка)", 2),
            context,
        );
        assert!(common.complete, "{common:?}");
        assert_eq!(
            common.value,
            [SymbolLocation {
                at: COMMON_AT.to_string(),
                range: Some(range(0, 10, 17)),
            }]
        );
    });
    assert!(
        provider.queries.lock().unwrap().is_empty(),
        "names Unica resolves never reach the provider"
    );
}

#[test]
fn a_saved_module_asks_the_provider_by_position_and_an_edited_one_by_name() {
    let environment = Environment;
    let provider = StubProvider::default();
    let service = SymbolService::new(Some(&provider), &environment);
    let position = position_of(TEXT, "Метод(", 1);
    with_context(|context| {
        let saved = service.definition(&document(TEXT, true), position, context);
        assert_eq!(
            saved.value,
            [SymbolLocation {
                at: "main:CommonModule.Внешний".to_string(),
                range: Some(range(4, 10, 15)),
            }]
        );
        assert!(!saved.complete);
        assert_eq!(saved.reasons, [Incompleteness::Unaddressable(1)]);

        let edited = service.hover(&document(TEXT, false), position, context);
        let info = edited.value.as_ref().unwrap();
        assert_eq!(
            info.signature.as_deref(),
            Some("Процедура Метод(Значение) Экспорт")
        );
        assert!(edited.reasons.contains(&Incompleteness::Stale));
        assert!(!edited.complete);
    });
    assert_eq!(
        *provider.queries.lock().unwrap(),
        [
            SymbolQuery::At {
                path: MODULE_PATH.to_string(),
                position,
            },
            SymbolQuery::Named {
                name: "Внешний.Метод".to_string(),
            },
        ]
    );
}

#[test]
fn references_and_calls_join_the_module_text_with_the_provider() {
    let environment = Environment;
    let provider = StubProvider::default();
    let service = SymbolService::new(Some(&provider), &environment);
    with_context(|context| {
        let method = position_of(TEXT, "Ф(А)", 0);
        let references = service.references(&document(TEXT, true), method, context);
        assert!(references.complete, "{references:?}");
        // Two places in the module from its text, one elsewhere from the
        // provider; the provider's place in this module is not doubled.
        assert_eq!(references.value.len(), 3, "{references:?}");
        assert_eq!(references.value[2].at, COMMON_AT);

        let caller = position_of(TEXT, "Вызов(", 0);
        let outgoing = service.calls(
            &document(TEXT, true),
            caller,
            CallDirection::Outgoing,
            context,
        );
        let targets: Vec<_> = outgoing
            .value
            .iter()
            .map(|location| location.at.as_str())
            .collect();
        assert_eq!(targets, [MODULE_AT, COMMON_AT]);
        assert!(!outgoing.complete, "the provider capped its answer");

        let incoming = service.calls(
            &document(TEXT, true),
            method,
            CallDirection::Incoming,
            context,
        );
        assert_eq!(
            incoming.value[0],
            SymbolLocation {
                at: MODULE_AT.to_string(),
                range: Some(range(0, 10, 15)),
            }
        );
    });
}

#[test]
fn an_answer_names_no_provider_and_a_failed_provider_is_a_reason() {
    struct Failing;
    impl SymbolProvider for Failing {
        fn symbol(
            &self,
            _: &SymbolQuery,
            _: &SymbolRequestContext<'_>,
        ) -> Result<Option<ProviderSymbol>, String> {
            Err("engine stopped".to_string())
        }
        fn references(
            &self,
            _: &SymbolQuery,
            _: &SymbolRequestContext<'_>,
        ) -> Result<ProviderPlaces, String> {
            Err("engine stopped".to_string())
        }
        fn calls(
            &self,
            _: &SymbolQuery,
            _: CallDirection,
            _: &SymbolRequestContext<'_>,
        ) -> Result<ProviderPlaces, String> {
            Err("engine stopped".to_string())
        }
    }
    let environment = Environment;
    let failing = Failing;
    let service = SymbolService::new(Some(&failing), &environment);
    let answer = with_context(|context| {
        service.definition(
            &document(TEXT, true),
            position_of(TEXT, "Метод(", 1),
            context,
        )
    });
    assert!(answer.value.is_empty());
    assert_eq!(
        answer.reasons,
        [Incompleteness::ProviderUnavailable(
            "engine stopped".to_string()
        )]
    );

    let provider = StubProvider::default();
    let service = SymbolService::new(Some(&provider), &environment);
    let answer = with_context(|context| {
        service.hover(
            &document(TEXT, true),
            position_of(TEXT, "Метод(", 1),
            context,
        )
    });
    let wire = serde_json::to_string(&answer).unwrap();
    assert!(!wire.contains("Outside/"), "a provider path leaked: {wire}");
    assert!(!wire.contains(".bsl"), "a file path leaked: {wire}");
}
