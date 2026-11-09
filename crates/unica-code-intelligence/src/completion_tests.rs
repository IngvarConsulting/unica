use super::*;

struct FixtureSources {
    platform: Vec<PlatformMember>,
    stale: bool,
    form: bool,
}

impl CompletionSources for FixtureSources {
    fn platform_members(&self) -> &[PlatformMember] {
        &self.platform
    }

    fn metadata_names(&self, kind: &str) -> Vec<String> {
        match kind {
            "CommonModule" => vec!["ОбщМод".to_owned()],
            "Catalog" => vec!["Валюты".to_owned(), "Номенклатура".to_owned()],
            _ => Vec::new(),
        }
    }

    fn common_module_text(&self, name: &str) -> Option<String> {
        (name == "ОбщМод").then(|| {
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

fn sources() -> FixtureSources {
    FixtureSources {
        platform: vec![PlatformMember {
            name: "Сообщить".to_owned(),
            method: true,
            parameters: vec!["ТекстСообщения".to_owned(), "Статус".to_owned()],
        }],
        stale: false,
        form: false,
    }
}

fn after(text: &str, needle: &str) -> usize {
    text.find(needle).unwrap() + needle.len()
}

fn labels(completion: &Completion) -> Vec<&str> {
    completion
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect()
}

#[test]
fn completion_offers_scope_names_metadata_and_object_members() {
    let sources = sources();
    let at = OBJECT_MODULE.rfind("\tСу\n").unwrap() + "\tСу".len();
    let answer = completion(OBJECT_MODULE, at, &sources).unwrap();
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
        assert!(names.contains(&expected), "{expected} in {names:?}");
    }
    // Locals come before module names and the global context.
    let rank = |label: &str| {
        answer
            .items
            .iter()
            .find(|item| item.label == label)
            .unwrap()
            .rank
    };
    assert!(rank("Сумма") < rank("Пересчитать") && rank("Пересчитать") < rank("Сообщить"));
    let message = answer
        .items
        .iter()
        .find(|item| item.label == "Сообщить")
        .unwrap();
    assert_eq!(
        message.detail.as_deref(),
        Some("Сообщить(ТекстСообщения, Статус)")
    );
    // The item replaces the typed «Су».
    assert_eq!(&OBJECT_MODULE[answer.replaces.clone()], "Су");

    let members = |needle: &str| {
        completion(OBJECT_MODULE, after(OBJECT_MODULE, needle), &sources)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>()
    };
    assert_eq!(members("Справочники.Ном"), ["Валюты", "Номенклатура"]);
    assert_eq!(members("\tОбщМод."), ["Метод"]);
    assert_eq!(members("\tЭтотОбъект."), ["Артикул", "Штрихкоды"]);
    assert_eq!(members("\tЭлемент."), ["Артикул", "Штрихкоды"]);
    assert_eq!(members("\t\tСтрока."), ["Штрихкод", "Упаковка"]);

    // No completion in a comment; a request no longer wanted answers nothing.
    assert_eq!(
        completion(OBJECT_MODULE, after(OBJECT_MODULE, "// Сум"), &sources),
        None
    );
    let stale = FixtureSources {
        stale: true,
        ..self::sources()
    };
    assert_eq!(
        completion(OBJECT_MODULE, after(OBJECT_MODULE, "\tОбщМод."), &stale),
        None
    );
}

#[test]
fn a_form_module_reaches_its_object_as_object_not_as_its_own_names() {
    let sources = FixtureSources {
        form: true,
        ..sources()
    };
    let text = "Процедура П()\n\tОбъект.\n\tА\nКонецПроцедуры\n";
    let members = completion(text, after(text, "Объект."), &sources).unwrap();
    assert_eq!(labels(&members), ["Артикул", "Штрихкоды"]);
    let names = completion(text, after(text, "\tА"), &sources).unwrap();
    assert!(!labels(&names).contains(&"Артикул"));
}

#[test]
fn an_item_names_its_method_and_resolves_its_description() {
    let sources = sources();
    let answer = completion(OBJECT_MODULE, after(OBJECT_MODULE, "\tОбщМод."), &sources).unwrap();
    let item = &answer.items[0];
    let method = item.method.clone().unwrap();
    assert_eq!(
        method,
        CompletionMethod {
            name: "Метод".to_owned(),
            common_module: Some("ОбщМод".to_owned()),
        }
    );
    assert_eq!(item.detail.as_deref(), Some("Метод(Первый) Экспорт"));
    assert_eq!(
        method_description(&sources.common_module_text("ОбщМод").unwrap(), &method).as_deref(),
        Some("Описание метода.")
    );
    // A module's own method is found whether exported or not.
    let own = CompletionMethod {
        name: "Пересчитать".to_owned(),
        common_module: None,
    };
    assert_eq!(
        method_description(OBJECT_MODULE, &own).as_deref(),
        Some("Пересчитывает строки.")
    );
}
