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
use unica_source_contract::proposal::{SupportLock, SupportLockKind};

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

/// The answer the person chose, when `call` is exactly one of the offered
/// answers to the question about `lock`.
pub(super) fn chosen_answer(lock: &SupportLock, call: &Map<String, Value>) -> Option<String> {
    support_choice(lock, call)?
        .options
        .into_iter()
        .find(|option| option.arguments == *call)
        .map(|option| option.id)
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
