use super::*;

/// Комментарий стандарта: описание, параметры с типами и описаниями,
/// возвращаемое значение и пример приходят полями, а не одним текстом.
#[test]
fn a_standard_comment_arrives_by_its_sections() {
    let comment = "// Создаёт копию таблицы.\n\
        //\n\
        // Параметры:\n\
        //   Строки - Массив из Строка - строки для копирования.\n\
        //   Флаг - Булево - признак, который\n\
        //     занимает две строки.\n\
        //\n\
        // Возвращаемое значение:\n\
        //   ТаблицаЗначений - копия.\n\
        //\n\
        // Пример:\n\
        //   Копия = Скопировать(Строки);\n";
    let doc = parse(comment);

    assert_eq!(
        doc.description.parsed().map(String::as_str),
        Some("Создаёт копию таблицы.")
    );
    let parameters = doc.parameters.parsed().expect("параметры разобраны");
    assert_eq!(parameters[0].name, "Строки");
    assert_eq!(parameters[0].types, ["Массив из Строка"]);
    assert_eq!(parameters[0].description, "строки для копирования.");
    // Строка глубже продолжает описание записи, а не открывает новую.
    assert_eq!(parameters[1].name, "Флаг");
    assert_eq!(
        parameters[1].description,
        "признак, который занимает две строки."
    );
    let returns = doc.returns.parsed().expect("возврат разобран");
    assert_eq!(returns.types, ["ТаблицаЗначений"]);
    assert_eq!(returns.description, "копия.");
    assert_eq!(
        doc.example.parsed().map(String::as_str),
        Some("   Копия = Скопировать(Строки);")
    );
    // Сырой текст остаётся полным рядом с полями.
    assert!(doc.text.starts_with(" Создаёт копию таблицы."));
    assert!(doc.text.contains("Возвращаемое значение:"));
}

/// Альтернативный тип записи и ключи коллекции: `- Тип - …` добавляет тип
/// предыдущей записи, `* Ключ - Тип - …` описывает ключ, `**` — ключ ключа.
#[test]
fn alternative_types_and_collection_keys_belong_to_their_entry() {
    let comment = "// Параметры:\n\
        //   Ссылка - СправочникСсылка.Валюты - валюта.\n\
        //          - Строка - её код.\n\
        //   Настройки - Структура:\n\
        //     * Период - Дата - начало.\n\
        //     * Отбор - Структура:\n\
        //       ** Склад - СправочникСсылка.Склады - склад отбора.\n";
    let doc = parse(comment);
    let parameters = doc.parameters.parsed().expect("параметры разобраны");

    assert_eq!(
        parameters[0].types,
        ["СправочникСсылка.Валюты", "Строка"],
        "альтернативный тип принадлежит своей записи: {parameters:?}"
    );
    assert_eq!(parameters[0].type_text, "СправочникСсылка.Валюты; Строка");
    assert_eq!(parameters[1].name, "Настройки");
    assert_eq!(parameters[1].keys[0].name, "Период");
    assert_eq!(parameters[1].keys[0].types, ["Дата"]);
    assert_eq!(parameters[1].keys[1].name, "Отбор");
    assert_eq!(parameters[1].keys[1].keys[0].name, "Склад");
    assert_eq!(
        parameters[1].keys[1].keys[0].types,
        ["СправочникСсылка.Склады"]
    );
}

/// Нестандартный комментарий остаётся текстом без потерь: разделов в нём
/// нет, и разбор не выдумывает ни параметров, ни возврата.
#[test]
fn a_comment_without_sections_keeps_its_whole_text_and_invents_nothing() {
    let doc = parse("// Проверяет заказ.\n// Вызывать только из обработчика.\n");

    assert_eq!(
        doc.description.parsed().map(String::as_str),
        Some("Проверяет заказ.\n Вызывать только из обработчика.")
    );
    assert!(doc.parameters.is_empty(), "{:?}", doc.parameters);
    assert!(doc.returns.is_empty(), "{:?}", doc.returns);
    assert!(doc.example.is_empty());
    assert_eq!(
        doc.text,
        " Проверяет заказ.\n Вызывать только из обработчика."
    );
}

/// Раздел, который в комментарии есть и не разобран, не становится пустым:
/// иначе «параметров нет» неотличимо от «параметры не разобрались», и вызов
/// собирается без обязательных аргументов.
#[test]
fn a_parameter_section_without_entries_is_unparsed_not_empty() {
    let doc = parse(
        "// Печатает заказ.\n\
         //\n\
         // Параметры:\n\
         //   описаны в стандарте предприятия.\n",
    );

    let Field::Unparsed { reason, text } = &doc.parameters else {
        panic!("раздел есть и не разобран: {:?}", doc.parameters);
    };
    assert_eq!(reason.as_str(), "no-parameter-entry");
    assert!(
        text.contains("описаны в стандарте предприятия."),
        "сырой текст раздела остаётся при причине: {text}"
    );
    // Раздела «Возвращаемое значение» в комментарии нет — это пусто.
    assert!(doc.returns.is_empty());
}

/// Раздел, которого стандарт не называет, не теряется: он остаётся под
/// своим заголовком среди прочих разделов.
#[test]
fn a_section_the_standard_does_not_name_stays_among_the_other_sections() {
    let doc = parse(
        "// Отправляет письмо.\n\
         //\n\
         // Ограничения:\n\
         //   только на сервере.\n\
         //\n\
         // См. также:\n\
         //   ОбщегоНазначения.Отправить\n",
    );

    assert_eq!(
        doc.other_sections,
        [(
            "Ограничения".to_string(),
            "   только на сервере.".to_string()
        )]
    );
    assert_eq!(
        doc.see_also.parsed().map(Vec::as_slice),
        Some(["ОбщегоНазначения.Отправить".to_string()].as_slice())
    );
}

/// Возвращаемое значение нескольких типов: типы собираются, а описание
/// значения не теряется.
#[test]
fn a_result_of_several_types_keeps_every_type_and_its_description() {
    let doc = parse(
        "// Ищет валюту.\n\
         //\n\
         // Возвращаемое значение:\n\
         //   СправочникСсылка.Валюты - найденная валюта;\n\
         //   Неопределено - если не нашлась.\n",
    );

    let returns = doc.returns.parsed().expect("возврат разобран");
    assert_eq!(returns.types, ["СправочникСсылка.Валюты", "Неопределено"]);
    assert_eq!(
        returns.description, "найденная валюта; если не нашлась.",
        "описания обоих случаев остаются: {returns:?}"
    );
}
