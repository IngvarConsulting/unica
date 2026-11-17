//! BSL handler emission from required typed method facts.
use crate::projection::MethodKind;
use unica_bsl_parser::parse as parse_bsl_syntax;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
use unica_source_contract::ScriptVariant;
use unica_xml_format::text::{
    event_line_start, event_local_line_ending, reject_event_lone_cr, resolve_observed_line_ending,
    SourceTextSnapshot, Utf8Bom,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredEventDirective {
    None,
    Client,
    Server,
    /// Reading accepts `&НаСервере` and `&НаСервереБезКонтекста`; a new
    /// handler is emitted without form context.
    ServerOrServerNoContext,
}

impl RequiredEventDirective {
    pub const fn canonical(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Client => Some("&НаКлиенте"),
            Self::Server => Some("&НаСервере"),
            Self::ServerOrServerNoContext => Some("&НаСервереБезКонтекста"),
        }
    }
}

/// Имена областей модуля по стандарту 455 (структура модуля).
pub mod standard_region {
    pub const VARIABLES: &str = "ОписаниеПеременных";
    pub const PUBLIC: &str = "ПрограммныйИнтерфейс";
    pub const EVENT_HANDLERS: &str = "ОбработчикиСобытий";
    pub const INTERNAL: &str = "СлужебныйПрограммныйИнтерфейс";
    pub const PRIVATE: &str = "СлужебныеПроцедурыИФункции";
    pub const INITIALIZE: &str = "Инициализация";
    pub const FORM_EVENT_HANDLERS: &str = "ОбработчикиСобытийФормы";
    pub const FORM_HEADER_ITEMS_EVENT_HANDLERS: &str = "ОбработчикиСобытийЭлементовШапкиФормы";
    /// Начало имени области таблицы формы: за ним идёт имя таблицы.
    pub const FORM_TABLE_ITEMS_EVENT_HANDLERS: &str = "ОбработчикиСобытийЭлементовТаблицыФормы";
    pub const FORM_COMMANDS_EVENT_HANDLERS: &str = "ОбработчикиКомандФормы";
}

/// Стандартная область: русское и английское имя; у области таблицы формы
/// это начало имени, за которым идёт имя таблицы.
struct StandardRegion {
    ru: &'static str,
    en: &'static str,
    prefix: bool,
}

const fn region(ru: &'static str, en: &'static str) -> StandardRegion {
    StandardRegion {
        ru,
        en,
        prefix: false,
    }
}

/// Порядок областей модуля формы по стандарту 455.
const FORM_MODULE_REGIONS: [StandardRegion; 7] = [
    region(standard_region::VARIABLES, "Variables"),
    region(standard_region::FORM_EVENT_HANDLERS, "FormEventHandlers"),
    region(
        standard_region::FORM_HEADER_ITEMS_EVENT_HANDLERS,
        "FormHeaderItemsEventHandlers",
    ),
    StandardRegion {
        ru: standard_region::FORM_TABLE_ITEMS_EVENT_HANDLERS,
        en: "FormTableItemsEventHandlers",
        prefix: true,
    },
    region(
        standard_region::FORM_COMMANDS_EVENT_HANDLERS,
        "FormCommandsEventHandlers",
    ),
    region(standard_region::PRIVATE, "Private"),
    region(standard_region::INITIALIZE, "Initialize"),
];

/// Порядок областей модуля объекта и прочих модулей по стандарту 455.
const MODULE_REGIONS: [StandardRegion; 6] = [
    region(standard_region::VARIABLES, "Variables"),
    region(standard_region::PUBLIC, "Public"),
    region(standard_region::EVENT_HANDLERS, "EventHandlers"),
    region(standard_region::INTERNAL, "Internal"),
    region(standard_region::PRIVATE, "Private"),
    region(standard_region::INITIALIZE, "Initialize"),
];

impl StandardRegion {
    /// Совпадает ли имя области с этой стандартной, на любом из двух языков и
    /// без учёта регистра.
    fn matches(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        [self.ru, self.en].iter().any(|standard| {
            let standard = standard.to_lowercase();
            if self.prefix {
                name.starts_with(&standard)
            } else {
                name == standard
            }
        })
    }
}

/// Порядок стандарта, в который входит область обработчика `region`.
fn standard_order(region: &str) -> &'static [StandardRegion] {
    if FORM_MODULE_REGIONS[1..5]
        .iter()
        .any(|standard| standard.matches(region))
    {
        &FORM_MODULE_REGIONS
    } else {
        &MODULE_REGIONS
    }
}

/// Место области в порядке стандарта; `None` — область не стандартная.
fn standard_rank(order: &[StandardRegion], name: &str) -> Option<usize> {
    order.iter().position(|standard| standard.matches(name))
}

/// Английское имя стандартной области по русскому; область таблицы формы
/// сохраняет имя таблицы. Нестандартное имя остаётся как есть.
fn english_region_name(region: &str) -> String {
    FORM_MODULE_REGIONS
        .iter()
        .chain(MODULE_REGIONS.iter())
        .find_map(|standard| {
            if standard.prefix {
                region
                    .strip_prefix(standard.ru)
                    .map(|table| format!("{}{table}", standard.en))
            } else {
                (region == standard.ru).then(|| standard.en.to_string())
            }
        })
        .unwrap_or_else(|| region.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleInsertionPlan {
    Existing {
        offset: usize,
    },
    /// Новая область перед стандартной областью, которая идёт после неё.
    Insert {
        offset: usize,
    },
    Append {
        offset: usize,
    },
}
impl ModuleInsertionPlan {
    const fn offset(self) -> usize {
        match self {
            Self::Existing { offset } | Self::Insert { offset } | Self::Append { offset } => offset,
        }
    }
}
pub fn emit_event_module(
    current: Option<&[u8]>,
    handler: &str,
    signature: &str,
    method_kind: MethodKind,
    directive: RequiredEventDirective,
    region: &str,
) -> Result<Vec<u8>, ApplyPlanError> {
    emit_event_module_with_variant(
        current,
        handler,
        signature,
        method_kind,
        directive,
        region,
        ScriptVariant::Russian,
    )
}

/// The region is the standard 455 handler region of the event owner; only its
/// name and markers follow the source-set script profile. A missing region is
/// placed by the standard order of the regions already in the module, before
/// the first top-level standard region that comes after it, else at the end.
/// Existing text stays intact.
pub fn emit_event_module_with_variant(
    current: Option<&[u8]>,
    handler: &str,
    signature: &str,
    method_kind: MethodKind,
    directive: RequiredEventDirective,
    region: &str,
    script_variant: ScriptVariant,
) -> Result<Vec<u8>, ApplyPlanError> {
    let order = standard_order(region);
    let (region, region_start, region_end) = match script_variant {
        ScriptVariant::Russian => (region.to_string(), "#Область", "#КонецОбласти"),
        ScriptVariant::English => (english_region_name(region), "#Region", "#EndRegion"),
    };
    let region = region.as_str();
    let target_rank = standard_rank(order, region);
    let owned_default;
    let bytes = match current {
        Some(bytes) => bytes,
        None => {
            owned_default = Vec::new();
            &owned_default
        }
    };
    let snapshot =
        SourceTextSnapshot::from_bytes(bytes).map_err(|error| invalid_source(error.to_string()))?;
    reject_event_lone_cr(&snapshot).map_err(invalid_source)?;
    let text = snapshot.text();
    let parsed = parse_bsl_syntax(text);
    if !parsed.errors().is_empty() {
        return Err(invalid_source(format!(
            "BSL parser reported {} diagnostic(s)",
            parsed.errors().len()
        )));
    }
    let handler_folded = handler.to_lowercase();
    let matching_methods = parsed
        .methods()
        .into_iter()
        .filter_map(|method| method.name)
        .filter(|name| name.to_lowercase() == handler_folded)
        .count();
    if matching_methods != 0 {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!(
                "event handler `{}` collides with an existing method",
                handler
            ),
        ));
    }

    let markers = parsed.region_markers();
    let canonical = region.to_lowercase();
    let mut stack = Vec::<(Option<String>, bool)>::new();
    let mut canonical_closes = Vec::new();
    // The first top-level standard region the standard puts after the new one.
    let mut next_standard = None;
    for marker in markers {
        if marker.is_start {
            let name = marker.name;
            let is_canonical = name
                .as_deref()
                .is_some_and(|name| name.to_lowercase() == canonical);
            if is_canonical && !stack.is_empty() {
                return Err(invalid_source("canonical handler region is nested"));
            }
            if stack.is_empty() && next_standard.is_none() {
                let rank = name.as_deref().and_then(|name| standard_rank(order, name));
                if let (Some(rank), Some(target)) = (rank, target_rank) {
                    if rank > target {
                        next_standard = Some(event_line_start(text, marker.range.start));
                    }
                }
            }
            stack.push((name, is_canonical));
        } else if marker.is_end {
            let Some((_, is_canonical)) = stack.pop() else {
                return Err(invalid_source("BSL region close marker has no opener"));
            };
            if is_canonical {
                canonical_closes.push(marker.range.start);
            }
        }
    }
    if !stack.is_empty() {
        return Err(invalid_source("BSL region is unclosed"));
    }
    if canonical_closes.len() > 1 {
        return Err(invalid_source("canonical handler region is duplicated"));
    }

    let insertion_plan = match (canonical_closes.first().copied(), next_standard) {
        (Some(marker_offset), _) => ModuleInsertionPlan::Existing {
            offset: event_line_start(text, marker_offset),
        },
        (None, Some(offset)) => ModuleInsertionPlan::Insert { offset },
        (None, None) => ModuleInsertionPlan::Append { offset: text.len() },
    };
    let eol = event_local_line_ending(text, insertion_plan.offset()).map_err(invalid_source)?;
    let eol = resolve_observed_line_ending(&snapshot, eol)
        .map_err(|error| invalid_source(error.to_string()))?
        .as_str();
    let method = render_method(signature, method_kind, directive, eol);
    let mut body = text.to_string();
    match insertion_plan {
        ModuleInsertionPlan::Existing { offset } => {
            body.insert_str(offset, &format!("{method}{eol}"));
        }
        ModuleInsertionPlan::Insert { offset } => {
            body.insert_str(
                offset,
                &format!("{region_start} {region}{eol}{eol}{method}{eol}{region_end}{eol}{eol}"),
            );
        }
        ModuleInsertionPlan::Append { .. } => {
            if !body.is_empty() {
                if !(body.ends_with('\n') || body.ends_with('\r')) {
                    body.push_str(eol);
                }
                body.push_str(eol);
            }
            body.push_str(&format!(
                "{region_start} {region}{eol}{eol}{method}{eol}{region_end}{eol}"
            ));
        }
    }
    let reparsed = parse_bsl_syntax(&body);
    if !reparsed.errors().is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "emitted BSL failed parser postcondition",
        ));
    }
    let mut output = Vec::with_capacity(body.len() + 3);
    if current.is_none() || snapshot.bom() == Utf8Bom::Present {
        output.extend_from_slice(b"\xef\xbb\xbf");
    }
    output.extend_from_slice(body.as_bytes());
    Ok(output)
}
fn render_method(
    signature: &str,
    method_kind: MethodKind,
    directive: RequiredEventDirective,
    eol: &str,
) -> String {
    let directive = directive
        .canonical()
        .map(|directive| format!("{directive}{eol}"))
        .unwrap_or_default();
    let end = match method_kind {
        MethodKind::Procedure => "КонецПроцедуры",
        MethodKind::Function => "КонецФункции",
    };
    format!("{directive}{}{eol}{eol}{end}{eol}", signature)
}

fn invalid_source(message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
}

#[cfg(test)]
mod script_variant_tests {
    use super::*;

    #[test]
    fn english_form_handler_uses_the_existing_english_region() {
        let text = b"#Region FormEventHandlers\r\n\r\n#EndRegion\r\n";
        let output = emit_event_module_with_variant(
            Some(text),
            "Opened",
            "Procedure Opened()",
            MethodKind::Procedure,
            RequiredEventDirective::Client,
            "ОбработчикиСобытийФормы",
            ScriptVariant::English,
        )
        .unwrap();
        let output = std::str::from_utf8(&output).unwrap();
        assert_eq!(output.matches("#Region FormEventHandlers").count(), 1);
        assert!(output.find("Opened()").unwrap() < output.find("#EndRegion").unwrap());
        assert!(!output.contains("#Область"));
        assert!(!output.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn english_form_handler_creates_an_english_region_without_rewriting_existing_text() {
        let text = "// comment\n";
        let output = emit_event_module_with_variant(
            Some(text.as_bytes()),
            "Clicked",
            "Procedure Clicked()",
            MethodKind::Procedure,
            RequiredEventDirective::Client,
            "ОбработчикиКомандФормы",
            ScriptVariant::English,
        )
        .unwrap();
        let output = std::str::from_utf8(&output).unwrap();
        assert!(output.starts_with(text));
        assert!(output.contains("#Region FormCommandsEventHandlers\n"));
        assert!(output.contains("#EndRegion\n"));
    }

    fn emit(text: &str, handler: &str, region: &str, variant: ScriptVariant) -> String {
        let output = emit_event_module_with_variant(
            Some(text.as_bytes()),
            handler,
            &format!("Процедура {handler}(Элемент)"),
            MethodKind::Procedure,
            RequiredEventDirective::Client,
            region,
            variant,
        )
        .unwrap();
        String::from_utf8(output)
            .unwrap()
            .trim_start_matches('\u{feff}')
            .to_string()
    }

    fn regions(text: &str) -> Vec<&str> {
        text.lines()
            .filter_map(|line| {
                line.strip_prefix("#Область ")
                    .or_else(|| line.strip_prefix("#Region "))
            })
            .collect()
    }

    /// Отсутствующая область обработчиков встаёт на своё место в порядке
    /// стандарта 455: перед первой стандартной областью, которая идёт после
    /// неё. Область каждой таблицы формы своя, и новая таблица встаёт за уже
    /// существующими. Прочий текст модуля не меняется.
    /// rule: INV.SOURCE.EVENT-IMPLEMENTATION-PAIR
    #[test]
    fn a_missing_handler_region_takes_its_place_in_the_standard_order() {
        let module = "#Область ОписаниеПеременных\n\n#КонецОбласти\n\n\
             #Область ОбработчикиСобытийФормы\n\n#КонецОбласти\n\n\
             #Область Моя\n\n#КонецОбласти\n\n\
             #Область ОбработчикиКомандФормы\n\n#КонецОбласти\n\n\
             #Область СлужебныеПроцедурыИФункции\n\n#КонецОбласти\n";
        let header = emit(
            module,
            "ПолеПриИзменении",
            "ОбработчикиСобытийЭлементовШапкиФормы",
            ScriptVariant::Russian,
        );
        assert_eq!(
            regions(&header),
            [
                "ОписаниеПеременных",
                "ОбработчикиСобытийФормы",
                "Моя",
                "ОбработчикиСобытийЭлементовШапкиФормы",
                "ОбработчикиКомандФормы",
                "СлужебныеПроцедурыИФункции",
            ]
        );
        assert_eq!(header.replace("#Область ОбработчикиСобытийЭлементовШапкиФормы\n\n&НаКлиенте\nПроцедура ПолеПриИзменении(Элемент)\n\nКонецПроцедуры\n\n#КонецОбласти\n\n", ""), module);

        let goods = emit(
            &header,
            "ТоварыПриИзменении",
            "ОбработчикиСобытийЭлементовТаблицыФормыТовары",
            ScriptVariant::Russian,
        );
        let payments = emit(
            &goods,
            "ОплатаПриИзменении",
            "ОбработчикиСобытийЭлементовТаблицыФормыОплата",
            ScriptVariant::Russian,
        );
        assert_eq!(
            regions(&payments),
            [
                "ОписаниеПеременных",
                "ОбработчикиСобытийФормы",
                "Моя",
                "ОбработчикиСобытийЭлементовШапкиФормы",
                "ОбработчикиСобытийЭлементовТаблицыФормыТовары",
                "ОбработчикиСобытийЭлементовТаблицыФормыОплата",
                "ОбработчикиКомандФормы",
                "СлужебныеПроцедурыИФункции",
            ]
        );
    }

    /// Без стандартных областей после нужной новая область дописывается в
    /// конец; в модуле объекта действует свой порядок стандарта.
    /// rule: INV.SOURCE.EVENT-IMPLEMENTATION-PAIR
    #[test]
    fn a_handler_region_goes_to_the_end_only_when_no_standard_region_follows_it() {
        let module = "#Область ОписаниеПеременных\n\n#КонецОбласти\n";
        let output = emit(
            module,
            "ПриОткрытии",
            "ОбработчикиСобытийФормы",
            ScriptVariant::Russian,
        );
        assert!(output.starts_with(module));
        assert_eq!(
            regions(&output),
            ["ОписаниеПеременных", "ОбработчикиСобытийФормы"]
        );

        let object = "#Область ПрограммныйИнтерфейс\n\n#КонецОбласти\n\n\
             #Область СлужебныйПрограммныйИнтерфейс\n\n#КонецОбласти\n";
        let output = emit(
            object,
            "ПриЗаписи",
            "ОбработчикиСобытий",
            ScriptVariant::Russian,
        );
        assert_eq!(
            regions(&output),
            [
                "ПрограммныйИнтерфейс",
                "ОбработчикиСобытий",
                "СлужебныйПрограммныйИнтерфейс",
            ]
        );
    }

    /// Английский вариант: область таблицы формы — FormTableItemsEventHandlers
    /// с именем таблицы, область шапки — FormHeaderItemsEventHandlers; имеющиеся
    /// английские области узнаются для порядка.
    /// rule: INV.SOURCE.EVENT-IMPLEMENTATION-PAIR
    #[test]
    fn english_handler_regions_follow_the_standard_names_and_order() {
        let module = "#Region FormEventHandlers\n\n#EndRegion\n\n\
             #Region Private\n\n#EndRegion\n";
        let table = emit(
            module,
            "GoodsOnChange",
            "ОбработчикиСобытийЭлементовТаблицыФормыGoods",
            ScriptVariant::English,
        );
        let header = emit(
            &table,
            "FieldOnChange",
            "ОбработчикиСобытийЭлементовШапкиФормы",
            ScriptVariant::English,
        );
        assert_eq!(
            regions(&header),
            [
                "FormEventHandlers",
                "FormHeaderItemsEventHandlers",
                "FormTableItemsEventHandlersGoods",
                "Private",
            ]
        );
        assert!(!header.contains("#Область"));
    }
}
