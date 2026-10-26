use super::*;
use std::sync::Mutex;
use std::time::Duration;

const MODULE_AT: &str = "main:Catalog.Валюты.Module.Manager";
const MODULE_PATH: &str = "Catalogs/Валюты/Ext/ManagerModule.bsl";
const COMMON_AT: &str = "main:CommonModule.Общий";
const COMMON_PATH: &str = "CommonModules/Общий/Ext/Module.bsl";
const COMMON_TEXT: &str = "Процедура Сделать(Значение) Экспорт\nКонецПроцедуры\n\nПроцедура Служебная()\nКонецПроцедуры\n";

const TEXT: &str = "Процедура Вызов(Параметр) Экспорт\n\tРезультат = Ф(Параметр);\n\tСсылка = Справочники.Валюты.ПустаяСсылка();\n\tОбщий.Сделать(Ссылка);\n\tВнешний.Метод(Ссылка.Код);\n\tЗапрос.Выполнить().Выбрать();\nКонецПроцедуры\n\nФункция Ф(А)\n\tВозврат А;\nКонецФункции\n";

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
            ("CommonModule", "Общий", None) => Some(COMMON_AT.to_string()),
            _ => None,
        }
    }

    fn module_text(&self, at: &str) -> Option<String> {
        (at == COMMON_AT).then(|| COMMON_TEXT.to_string())
    }
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

/// A provider that is not `bsl-analyzer`: the service must accept it as is.
#[derive(Default)]
struct StubProvider {
    queries: Mutex<Vec<SymbolQuery>>,
}

impl SymbolProvider for StubProvider {
    fn symbol(
        &self,
        query: &SymbolQuery,
        _context: &SymbolRequestContext<'_>,
    ) -> Result<Option<ProviderSymbol>, ProviderFailure> {
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
            complete: true,
        }))
    }

    fn references(
        &self,
        query: &SymbolQuery,
        _context: &SymbolRequestContext<'_>,
    ) -> Result<ProviderPlaces, ProviderFailure> {
        self.queries.lock().unwrap().push(query.clone());
        Ok(ProviderPlaces {
            places: vec![
                ProviderPlace::Code {
                    path: COMMON_PATH.to_string(),
                    range: range(7, 1, 2),
                },
                ProviderPlace::Code {
                    path: MODULE_PATH.to_string(),
                    range: range(3, 7, 14),
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
    ) -> Result<ProviderPlaces, ProviderFailure> {
        self.queries.lock().unwrap().push(query.clone());
        Ok(ProviderPlaces {
            places: vec![
                ProviderPlace::Code {
                    path: COMMON_PATH.to_string(),
                    range: range(0, 10, 17),
                },
                ProviderPlace::Code {
                    path: MODULE_PATH.to_string(),
                    range: range(0, 10, 15),
                },
            ],
            complete: false,
        })
    }
}

fn document<'a>(text: &'a str, saved: &'a str) -> SymbolDocument<'a> {
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
        deadline: ProviderDeadline::from_budget(Duration::from_secs(30)),
        cancellation: &cancellation,
    })
}

fn addresses(answer: &SymbolAnswer<Vec<SymbolLocation>>) -> Vec<&str> {
    answer
        .value
        .iter()
        .map(|location| location.at.as_str())
        .collect()
}

#[test]
fn module_names_are_answered_from_its_own_text_without_a_provider() {
    let environment = Environment;
    let service = SymbolService::new(None, &environment);
    with_context(|context| {
        let call = position_of(TEXT, "Ф(Параметр)", 0);
        let definition = service.definition(&document(TEXT, TEXT), call, context);
        assert!(definition.complete, "{definition:?}");
        assert_eq!(
            definition.value,
            [SymbolLocation {
                at: MODULE_AT.to_string(),
                range: Some(range(8, 8, 9)),
            }]
        );

        let parameter = position_of(TEXT, "Параметр);", 0);
        let references = service.references(&document(TEXT, TEXT), parameter, context);
        assert!(references.complete);
        assert_eq!(references.value.len(), 2);

        let hover = service.hover(&document(TEXT, TEXT), parameter, context);
        assert_eq!(hover.value.as_ref().unwrap().kind, SymbolKind::Parameter);

        // A method visible elsewhere is answered here only.
        let method = service.references(&document(TEXT, TEXT), call, context);
        assert_eq!(method.value.len(), 2);
        assert_eq!(method.reasons, [Incompleteness::OtherModules]);
    });
}

#[test]
fn metadata_and_common_module_names_resolve_to_unica_nodes() {
    let environment = Environment;
    let provider = StubProvider::default();
    let service = SymbolService::new(Some(&provider), &environment);
    let edited = format!("{TEXT}\n");
    with_context(|context| {
        let catalog = service.definition(
            &document(TEXT, TEXT),
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
            &document(&edited, TEXT),
            position_of(&edited, "Сделать(Ссылка)", 2),
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
fn a_saved_module_asks_by_position_and_an_edited_one_by_saved_declaration_or_name() {
    let environment = Environment;
    let provider = StubProvider::default();
    let service = SymbolService::new(Some(&provider), &environment);
    let position = position_of(TEXT, "Метод(", 1);
    let edited = format!("// правка\n{TEXT}");
    with_context(|context| {
        let saved = service.definition(&document(TEXT, TEXT), position, context);
        assert_eq!(
            saved.value,
            [SymbolLocation {
                at: "main:CommonModule.Внешний".to_string(),
                range: Some(range(4, 10, 15)),
            }]
        );
        assert_eq!(saved.reasons, [Incompleteness::Unaddressable(1)]);

        let unknown = service.hover(
            &document(&edited, TEXT),
            position_of(&edited, "Метод(", 1),
            context,
        );
        let info = unknown.value.as_ref().unwrap();
        assert_eq!(
            info.signature.as_deref(),
            Some("Процедура Метод(Значение) Экспорт")
        );
        assert!(unknown.reasons.contains(&Incompleteness::Stale));

        // The module's own method is asked where the saved text declares it,
        // never by its bare name, which another module may share.
        let own = service.calls(
            &document(&edited, TEXT),
            position_of(&edited, "Ф(А)", 0),
            CallDirection::Incoming,
            context,
        );
        assert!(own.reasons.contains(&Incompleteness::Stale));
    });
    let queries = provider.queries.lock().unwrap();
    assert_eq!(
        queries[..2],
        [
            SymbolQuery::At {
                path: MODULE_PATH.to_string(),
                position,
            },
            SymbolQuery::Named {
                name: "Внешний.Метод".to_string(),
                path: MODULE_PATH.to_string(),
            },
        ]
    );
    assert_eq!(
        queries[2],
        SymbolQuery::At {
            path: MODULE_PATH.to_string(),
            position: position_of(TEXT, "Ф(А)", 0),
        }
    );
}

#[test]
fn an_edited_buffer_asks_unica_resolved_names_by_name_and_marks_new_ones_stale() {
    let environment = Environment;
    let provider = StubProvider::default();
    let service = SymbolService::new(Some(&provider), &environment);
    let edited = format!("{TEXT}\nПроцедура Новая()\n\tНовая();\nКонецПроцедуры\n");
    with_context(|context| {
        let common = service.calls(
            &document(&edited, TEXT),
            position_of(&edited, "Сделать(Ссылка)", 2),
            CallDirection::Incoming,
            context,
        );
        assert!(!common.complete);
        assert!(common.reasons.contains(&Incompleteness::Stale));
        assert!(!common.value.is_empty(), "{common:?}");

        // The saved text has never seen this method: the buffer alone answers.
        let new = service.calls(
            &document(&edited, TEXT),
            position_of(&edited, "Новая();", 0),
            CallDirection::Incoming,
            context,
        );
        assert_eq!(new.reasons, [Incompleteness::Stale]);
    });
    assert_eq!(
        *provider.queries.lock().unwrap(),
        [SymbolQuery::Named {
            name: "Общий.Сделать".to_string(),
            path: MODULE_PATH.to_string(),
        }]
    );
}

/// References are this module's places only: the walk over the whole
/// configuration is never started by an editor request. Calls join the
/// provider's places to the module's own.
#[test]
fn references_stay_in_the_module_and_calls_join_the_provider() {
    let environment = Environment;
    let provider = StubProvider::default();
    let service = SymbolService::new(Some(&provider), &environment);
    with_context(|context| {
        let external = service.references(
            &document(TEXT, TEXT),
            position_of(TEXT, "Метод(", 1),
            context,
        );
        assert!(external.value.is_empty());
        assert_eq!(external.reasons, [Incompleteness::OtherModules]);

        let own = service.references(&document(TEXT, TEXT), position_of(TEXT, "Ф(А)", 0), context);
        assert_eq!(addresses(&own), [MODULE_AT, MODULE_AT]);
        assert_eq!(own.reasons, [Incompleteness::OtherModules]);

        let common = service.references(
            &document(TEXT, TEXT),
            position_of(TEXT, "Сделать(Ссылка)", 2),
            context,
        );
        assert_eq!(common.reasons, [Incompleteness::OtherModules]);
    });
    assert!(
        provider.queries.lock().unwrap().is_empty(),
        "references never reach the provider"
    );
    with_context(|context| {
        let outgoing = service.calls(
            &document(TEXT, TEXT),
            position_of(TEXT, "Вызов(", 0),
            CallDirection::Outgoing,
            context,
        );
        assert_eq!(addresses(&outgoing), [MODULE_AT, COMMON_AT]);
        assert_eq!(outgoing.reasons, [Incompleteness::Capped]);

        let incoming = service.calls(
            &document(TEXT, TEXT),
            position_of(TEXT, "Ф(А)", 0),
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
fn a_name_in_a_call_chain_and_an_unknown_catalog_go_to_the_provider() {
    let environment = Environment;
    let provider = StubProvider::default();
    let service = SymbolService::new(Some(&provider), &environment);
    let text = TEXT.replace("Справочники.Валюты", "Справочники.Пропавшие");
    with_context(|context| {
        let chained = service.hover(
            &document(TEXT, TEXT),
            position_of(TEXT, "Выбрать", 1),
            context,
        );
        assert!(
            chained.value.is_some(),
            "the provider is asked: {chained:?}"
        );
        let unknown = service.definition(
            &document(&text, &text),
            position_of(&text, "Пропавшие", 1),
            context,
        );
        assert!(
            !unknown.value.is_empty(),
            "the provider is asked: {unknown:?}"
        );
    });
    assert_eq!(provider.queries.lock().unwrap().len(), 2);
}

#[test]
fn an_answer_names_no_provider_and_a_failed_provider_is_a_unica_reason() {
    struct Failing;
    impl SymbolProvider for Failing {
        fn symbol(
            &self,
            _: &SymbolQuery,
            _: &SymbolRequestContext<'_>,
        ) -> Result<Option<ProviderSymbol>, ProviderFailure> {
            Err(ProviderFailure::Unavailable)
        }
        fn references(
            &self,
            _: &SymbolQuery,
            _: &SymbolRequestContext<'_>,
        ) -> Result<ProviderPlaces, ProviderFailure> {
            Err(ProviderFailure::Unavailable)
        }
        fn calls(
            &self,
            _: &SymbolQuery,
            _: CallDirection,
            _: &SymbolRequestContext<'_>,
        ) -> Result<ProviderPlaces, ProviderFailure> {
            Err(ProviderFailure::Unavailable)
        }
    }
    let environment = Environment;
    let failing = Failing;
    let service = SymbolService::new(Some(&failing), &environment);
    let answer = with_context(|context| {
        service.definition(
            &document(TEXT, TEXT),
            position_of(TEXT, "Метод(", 1),
            context,
        )
    });
    assert!(answer.value.is_empty());
    assert_eq!(answer.reasons, [Incompleteness::ProviderUnavailable]);

    let provider = StubProvider::default();
    let service = SymbolService::new(Some(&provider), &environment);
    let answer = with_context(|context| {
        service.hover(
            &document(TEXT, TEXT),
            position_of(TEXT, "Метод(", 1),
            context,
        )
    });
    let wire = serde_json::to_string(&answer).unwrap();
    assert!(!wire.contains(".bsl"), "a file path leaked: {wire}");
    assert_eq!(
        serde_json::to_value(Incompleteness::Unaddressable(2)).unwrap(),
        serde_json::json!({"kind": "unaddressable", "count": 2})
    );
}
