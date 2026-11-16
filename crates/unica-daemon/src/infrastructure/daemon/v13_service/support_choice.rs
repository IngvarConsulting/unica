//! Вопрос человеку о запрете поставщика в `unica.apply`
//! (INV.SAFETY.APPLY-SUPPORT-GATE, INV.SOURCE.SUPPORT-SEPARATE-APPLY).
//!
//! Отказ `deny` спрашивает человека, снять ли запрет. Вариант — тот же вызов
//! `apply` с отметкой согласия, привязанной к вопросу и выбранному варианту.
//! По такой отметке планировщик кладёт изменение поддержки этого объекта
//! и исходную правку в один план. Модель отметку подать не может: дверь
//! отклоняет вызов с ней до демона.

use serde_json::{json, Map, Value};
use unica_execution_contract::invocation::{HumanChoice, HumanChoiceOption};
use unica_source_contract::compatibility::CompatibilityMode;
use unica_source_contract::proposal::{ReadOnlyMode, SupportLock, SupportLockKind};

const APPLY_TOOL: &str = "unica.apply";
const EDITABLE: &str = "editable";
const OFF_SUPPORT: &str = "off-support";

/// The question about `lock` for the refused `call`, or none when no
/// support operation can lift it: a rule addresses a top-level object, never
/// the configuration root.
pub(super) fn support_choice(lock: &SupportLock, call: &Map<String, Value>) -> Option<HumanChoice> {
    if lock.owner == "Configuration" {
        return None;
    }
    let (code, question) = match lock.kind {
        SupportLockKind::ObjectLocked => (
            "support.object_locked",
            format!(
                "Объект «{}» набора «{}» на поддержке поставщика: правка запрещена.",
                lock.owner, lock.source_set
            ),
        ),
        SupportLockKind::ConfigurationLocked => (
            "support.configuration_locked",
            format!(
                "Конфигурация набора «{}» на поддержке поставщика без возможности изменения.",
                lock.source_set
            ),
        ),
        SupportLockKind::RemovalLocked => (
            "support.remove_locked",
            format!(
                "Объект «{}» на поддержке поставщика: удалить его можно только после снятия с поддержки.",
                lock.owner
            ),
        ),
    };
    let answers: &[(&str, &str, &str)] = match lock.kind {
        SupportLockKind::ObjectLocked | SupportLockKind::ConfigurationLocked => &[
            (
                EDITABLE,
                "Разрешить правку с сохранением поддержки",
                "при обновлении от поставщика по объекту возможны конфликты",
            ),
            (
                OFF_SUPPORT,
                "Снять объект с поддержки",
                "обновления поставщика по объекту перестанут приходить",
            ),
        ],
        SupportLockKind::RemovalLocked => &[(
            OFF_SUPPORT,
            "Снять с поддержки и удалить",
            "обновления поставщика по объекту прекратятся, объект будет удалён",
        )],
    };
    let options = answers
        .iter()
        .map(|(id, title, description)| HumanChoiceOption {
            id: (*id).to_string(),
            title: (*title).to_string(),
            description: (*description).to_string(),
            tool: APPLY_TOOL.to_string(),
            arguments: confirmed(code, &question, id, call),
            then: Vec::new(),
        })
        .collect();
    let cancel_description = (lock.kind != SupportLockKind::RemovalLocked)
        .then(|| "доработку можно внести в расширение".to_string());
    Some(HumanChoice {
        code: code.to_string(),
        question,
        data: Some(json!({"sourceSet": lock.source_set, "object": lock.owner})),
        options,
        cancel_description,
        proceeds_unasked: false,
    })
}

/// The call carrying the person's confirmation of one answer: the mark binds
/// the question, the answer and the arguments, so one answer never stands
/// for another and a changed call needs a new question.
fn confirmed(
    code: &str,
    question: &str,
    answer: &str,
    call: &Map<String, Value>,
) -> Map<String, Value> {
    HumanChoice::confirmation(
        code,
        format!("{question}\n{answer}"),
        "",
        "",
        APPLY_TOOL,
        call,
    )
    .options
    .remove(0)
    .arguments
}

/// The answer the person chose: `call` carries the mark of one of the offered
/// answers to the question about `lock`, among the marks of other questions
/// the person answered about the same call.
pub(super) fn chosen_answer(lock: &SupportLock, call: &Map<String, Value>) -> Option<String> {
    let choice = support_choice(lock, call)?;
    choice
        .options
        .into_iter()
        .find(|option| {
            HumanChoice::is_confirmed(
                &format!("{}\n{}", choice.question, option.id),
                APPLY_TOOL,
                call,
            )
        })
        .map(|option| option.id)
}

/// The question about a set read-only by its compatibility mode, asked with
/// the agreed words (INV.SOURCE.WRITABLE-PROFILE).
pub(super) fn mode_choice(
    read_only: &ReadOnlyMode,
    source_set: &str,
    call: &Map<String, Value>,
) -> HumanChoice {
    let question = mode_question(read_only, source_set);
    HumanChoice {
        code: MODE_CODE.to_string(),
        options: vec![HumanChoiceOption {
            id: RAISE_MODE.to_string(),
            title: "Повысить режим до 8.3.8 и внести правку".to_string(),
            description: "поведение конфигурации может измениться".to_string(),
            tool: APPLY_TOOL.to_string(),
            arguments: confirmed(MODE_CODE, &question, RAISE_MODE, call),
            then: Vec::new(),
        }],
        data: Some(json!({
            "sourceSet": source_set,
            "mode": human_mode(read_only.mode),
            "minimumMode": human_mode(minimum_editable_mode()),
        })),
        question,
        cancel_description: None,
        proceeds_unasked: false,
    }
}

/// Whether `call` carries the person's answer to raise the mode.
pub(super) fn mode_raise_chosen(
    read_only: &ReadOnlyMode,
    source_set: &str,
    call: &Map<String, Value>,
) -> bool {
    let question = mode_question(read_only, source_set);
    HumanChoice::is_confirmed(&format!("{question}\n{RAISE_MODE}"), APPLY_TOOL, call)
}

/// The apply request that raises the set's own mode to 8.3.8.
pub(super) fn mode_raise_request(read_only: &ReadOnlyMode, source_set: &str) -> Map<String, Value> {
    match json!({
        "at": format!("{source_set}:Configuration"),
        "ops": [{"op": "props.set", "args": {"values": {read_only.property.as_str(): minimum_editable_mode().to_string()}}}],
        "dryRun": true,
    }) {
        Value::Object(request) => request,
        _ => unreachable!("an apply request is an object"),
    }
}

const REMOVE_SUPPORT_CODE: &str = "support.state_unreadable";
const REMOVE_SUPPORT: &str = "remove-support";

fn remove_support_question(source_set: &str) -> String {
    format!("Состояние поддержки поставщика у набора «{source_set}» повреждено и не читается.")
}

/// The question about a vendor-support state that cannot be read, asked
/// with the agreed words (INV.SAFETY.APPLY-SUPPORT-GATE): the one way out
/// is taking the configuration off support entirely.
pub(super) fn remove_support_choice(source_set: &str, call: &Map<String, Value>) -> HumanChoice {
    let question = remove_support_question(source_set);
    HumanChoice {
        code: REMOVE_SUPPORT_CODE.to_string(),
        options: vec![HumanChoiceOption {
            id: REMOVE_SUPPORT.to_string(),
            title: "Снять с поддержки полностью".to_string(),
            description:
                "сведения о поставщике и правила объектов пропадут, обновления поставщика перестанут приходить"
                    .to_string(),
            tool: APPLY_TOOL.to_string(),
            arguments: confirmed(REMOVE_SUPPORT_CODE, &question, REMOVE_SUPPORT, call),
            then: Vec::new(),
        }],
        data: Some(json!({"sourceSet": source_set})),
        question,
        cancel_description: Some(
            "восстановите базу из резервной копии и выгрузите набор заново".to_string(),
        ),
        proceeds_unasked: false,
    }
}

/// Whether `call` carries the person's answer to take the configuration off
/// support entirely.
pub(super) fn remove_support_chosen(source_set: &str, call: &Map<String, Value>) -> bool {
    let question = remove_support_question(source_set);
    HumanChoice::is_confirmed(&format!("{question}\n{REMOVE_SUPPORT}"), APPLY_TOOL, call)
}

const MODE_CODE: &str = "source.compatibility_mode";
const RAISE_MODE: &str = "raise";

fn mode_question(read_only: &ReadOnlyMode, source_set: &str) -> String {
    format!(
        "Набор «{source_set}» в режиме совместимости {} можно только читать. Правки доступны с режима 8.3.8.",
        human_mode(read_only.mode)
    )
}

/// The oldest mode Unica edits, the one the answer raises the set to.
fn minimum_editable_mode() -> CompatibilityMode {
    CompatibilityMode::Version(unica_source_contract::compatibility::MINIMUM_EDITABLE_MODE)
}

/// `Version8_2_13` as the person reads it: `8.2.13`.
fn human_mode(mode: CompatibilityMode) -> String {
    let recorded = mode.to_string();
    recorded
        .strip_prefix("Version")
        .map_or(recorded.clone(), |version| version.replace('_', "."))
}

/// The apply requests that make the chosen change of support: editing of
/// the configuration first when it is disabled, then the rule of the object.
pub(super) fn support_requests(lock: &SupportLock, answer: &str) -> Vec<Map<String, Value>> {
    let mut requests = Vec::new();
    if lock.kind == SupportLockKind::ConfigurationLocked {
        requests.push(json!({
            "at": format!("{}:Configuration", lock.source_set),
            "ops": [{"op": "supportCapability.set", "args": {"values": {"enabled": true}}}],
            "dryRun": true,
        }));
    }
    let rule = if answer == EDITABLE {
        EDITABLE
    } else {
        OFF_SUPPORT
    };
    requests.push(json!({
        "at": format!("{}:{}", lock.source_set, lock.owner),
        "ops": [{"op": "supportRule.set", "args": {"values": {"rule": rule}}}],
        "dryRun": true,
    }));
    requests
        .into_iter()
        .map(|request| match request {
            Value::Object(request) => request,
            _ => unreachable!("an apply request is an object"),
        })
        .collect()
}

const FORMAT_OLD_CODE: &str = "source.format_old";
const FORMAT_MIXED_CODE: &str = "source.format_mixed";

/// The question about a set whose export format keeps it from an edit, with
/// the agreed words (INV.SOURCE.WRITABLE-PROFILE). Each answer re-exports the
/// set in one writable format and then makes the refused `call` again, both
/// in one `unica.apply`: a long re-export continues as one Task with the edit.
/// The choice is the person's consent to replace the files of the set, so the
/// re-export asks nothing more.
pub(super) fn format_choice(
    reexport: &unica_source_contract::proposal::FormatReexport,
    call: &Map<String, Value>,
) -> HumanChoice {
    use unica_source_contract::proposal::FormatReexportKind;
    use unica_source_designer::format_profile::{
        platform_line_for_export_format, WRITABLE_EXPORT_FORMATS,
    };
    let set = &reexport.source_set;
    let line = |format: &str| platform_line_for_export_format(format).unwrap_or_default();
    let answer = |id: String, title: String, format: &str| HumanChoiceOption {
        id,
        title,
        description: "файлы набора перезапишутся, несохранённые правки в них пропадут".to_string(),
        tool: APPLY_TOOL.to_string(),
        arguments: crate::infrastructure::daemon::v13_source_export::reexport_before_apply_call(
            call, set, format,
        ),
        then: Vec::new(),
    };
    match &reexport.kind {
        FormatReexportKind::Old { actual } => {
            let target = WRITABLE_EXPORT_FORMATS[0];
            HumanChoice {
                code: FORMAT_OLD_CODE.to_string(),
                question: format!(
                    "Набор «{set}» выгружен в формате {actual}. Правки доступны с формата {target} (платформа {}).",
                    line(target)
                ),
                data: Some(json!({
                    "sourceSet": set,
                    "format": actual,
                    "targetFormat": target,
                    "platformLine": line(target),
                })),
                options: vec![answer(
                    "reexport".to_string(),
                    format!("Перевыгрузить набор в формате {target}"),
                    target,
                )],
                cancel_description: None,
                proceeds_unasked: false,
            }
        }
        FormatReexportKind::Mixed { set_format, other } => HumanChoice {
            code: FORMAT_MIXED_CODE.to_string(),
            question: format!(
                "Корни набора «{set}» выгружены в разных форматах: {set_format} и {other}. Правка сохраняет один формат."
            ),
            data: Some(json!({"sourceSet": set, "formats": [set_format, other]})),
            options: [set_format, other]
                .into_iter()
                .map(|format| {
                    answer(
                        format!("reexport-{format}"),
                        format!("Перевыгрузить в формате {format} ({})", line(format)),
                        format,
                    )
                })
                .collect(),
            cancel_description: None,
            proceeds_unasked: false,
        },
    }
}

#[cfg(test)]
mod format_choice_tests {
    use super::*;
    use unica_source_contract::proposal::{FormatReexport, FormatReexportKind};

    /// A set whose roots record two writable formats is offered both, each
    /// with its platform line, in the agreed words; each answer re-exports the
    /// set in its format and makes the same edit again.
    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn a_mixed_set_is_offered_a_reexport_in_each_of_its_formats() {
        let call: Map<String, Value> = serde_json::from_value(json!({
            "at": "main:Catalog.Products",
            "ops": [{"op": "attribute.add", "args": {"items": [{"name": "Extra"}]}}]
        }))
        .unwrap();
        let choice = format_choice(
            &FormatReexport {
                source_set: "main".to_string(),
                kind: FormatReexportKind::Mixed {
                    set_format: "2.20".to_string(),
                    other: "2.22".to_string(),
                },
            },
            &call,
        );
        assert_eq!(choice.code, "source.format_mixed");
        assert_eq!(
            choice.question,
            "Корни набора «main» выгружены в разных форматах: 2.20 и 2.22. Правка сохраняет один формат."
        );
        let titles = choice
            .options
            .iter()
            .map(|option| option.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            titles,
            [
                "Перевыгрузить в формате 2.20 (8.3.27)",
                "Перевыгрузить в формате 2.22 (8.5.4)"
            ]
        );
        for (option, format) in choice.options.iter().zip(["2.20", "2.22"]) {
            assert_eq!(
                option.description,
                "файлы набора перезапишутся, несохранённые правки в них пропадут"
            );
            assert_eq!(option.tool, APPLY_TOOL);
            assert!(option.then.is_empty());
            let mut edit = option.arguments.clone();
            assert_eq!(
                edit.remove(unica_execution_contract::invocation::HUMAN_REEXPORT_KEY),
                Some(json!({"sourceSet": "main", "format": format}))
            );
            assert_eq!(edit, call);
        }
        assert_eq!(
            choice.data,
            Some(json!({"sourceSet": "main", "formats": ["2.20", "2.22"]}))
        );
    }
}
