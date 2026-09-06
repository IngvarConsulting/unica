//! Закрытый словарь отказов канонической поверхности.
//!
//! Код отказа — это тип, а не строка: литерал не может попасть на провод
//! мимо словаря, потому что конструкторы отказа принимают только
//! [`RefusalCode`]. Исход — метод кода, поэтому вторым словарём он не
//! становится и разойтись с кодами не может.
//!
//! Раскладка кодов по исходам обоснована в
//! `docs/design/2026-09-04-canonical-surface-distribution-design.md`.

/// Что делать дальше: кто действует следующим и над чем.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Преходящее: тот же самый вызов может позже пройти.
    RetryAsIs,
    /// Агент правит аргументы вызова.
    FixCall,
    /// Агент правит исходники.
    FixSource,
    /// Человек действует над средой.
    NeedsHuman,
    /// Маршрут не работает, альтернатива в `next`.
    GoElsewhere,
    /// Остановиться и назвать причину.
    DeadEnd,
}

impl Outcome {
    /// Значение исхода на проводе.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RetryAsIs => "retry",
            Self::FixCall => "fixCall",
            Self::FixSource => "fixSource",
            Self::NeedsHuman => "needsHuman",
            Self::GoElsewhere => "goElsewhere",
            Self::DeadEnd => "deadEnd",
        }
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Код отказа канонической поверхности.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefusalCode {
    /// `concurrent_change`
    ConcurrentChange,
    /// `source_selection_changed`
    SourceSelectionChanged,
    /// `deadline_exceeded`
    DeadlineExceeded,
    /// `provider_deadline`
    ProviderDeadline,
    /// `dependency_unavailable`
    DependencyUnavailable,
    /// `task_transport_failed`
    TaskTransportFailed,
    /// `task_session_closed`
    TaskSessionClosed,
    /// `bad_value`
    BadValue,
    /// `not_found`
    NotFound,
    /// `source_set_unknown`
    SourceSetUnknown,
    /// `target_not_found`
    TargetNotFound,
    /// `containment_denied`
    ContainmentDenied,
    /// `invalid_cursor`
    InvalidCursor,
    /// `unsupported_cursor`
    UnsupportedCursor,
    /// `unsupported_source`
    UnsupportedSource,
    /// `unsupported_filter`
    UnsupportedFilter,
    /// `incomparable_nodes`
    IncomparableNodes,
    /// `result_too_large`
    ResultTooLarge,
    /// `invalid_task_id`
    InvalidTaskId,
    /// `bad_wait_ms`
    BadWaitMs,
    /// `bad_task_arguments`
    BadTaskArguments,
    /// `revision_mismatch`
    RevisionMismatch,
    /// `stale_revision`
    StaleRevision,
    /// `stale_cursor`
    StaleCursor,
    /// `provider_limit_exceeded`
    ProviderLimitExceeded,
    /// `resource_absent`
    ResourceAbsent,
    /// `target_kind_unsupported`
    TargetKindUnsupported,
    /// `unsupported_section`
    UnsupportedSection,
    /// `unsupported_scope`
    UnsupportedScope,
    /// `invalid_source`
    InvalidSource,
    /// `postcondition_failed`
    PostconditionFailed,
    /// `ambiguous_source_format`
    AmbiguousSourceFormat,
    /// `rollback_incomplete`
    RollbackIncomplete,
    /// `invalid_state`
    InvalidState,
    /// `provider_unavailable`
    ProviderUnavailable,
    /// `task_backend_failed`
    TaskBackendFailed,
    /// `unsupported_operation`
    UnsupportedOperation,
    /// `profile_unsupported`
    ProfileUnsupported,
    /// `task_not_found`
    TaskNotFound,
    /// `task_expired`
    TaskExpired,
    /// `cancelled`
    Cancelled,
    /// `task_protocol_failed`
    TaskProtocolFailed,
    /// `task_projection_failed`
    TaskProjectionFailed,
    /// `provider_failed`
    ProviderFailed,
    /// `invalid_result`
    InvalidResult,
}

impl RefusalCode {
    /// Имя кода на проводе.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConcurrentChange => "concurrent_change",
            Self::SourceSelectionChanged => "source_selection_changed",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::ProviderDeadline => "provider_deadline",
            Self::DependencyUnavailable => "dependency_unavailable",
            Self::TaskTransportFailed => "task_transport_failed",
            Self::TaskSessionClosed => "task_session_closed",
            Self::BadValue => "bad_value",
            Self::NotFound => "not_found",
            Self::SourceSetUnknown => "source_set_unknown",
            Self::TargetNotFound => "target_not_found",
            Self::ContainmentDenied => "containment_denied",
            Self::InvalidCursor => "invalid_cursor",
            Self::UnsupportedCursor => "unsupported_cursor",
            Self::UnsupportedSource => "unsupported_source",
            Self::UnsupportedFilter => "unsupported_filter",
            Self::IncomparableNodes => "incomparable_nodes",
            Self::ResultTooLarge => "result_too_large",
            Self::InvalidTaskId => "invalid_task_id",
            Self::BadWaitMs => "bad_wait_ms",
            Self::BadTaskArguments => "bad_task_arguments",
            Self::RevisionMismatch => "revision_mismatch",
            Self::StaleRevision => "stale_revision",
            Self::StaleCursor => "stale_cursor",
            Self::ProviderLimitExceeded => "provider_limit_exceeded",
            Self::ResourceAbsent => "resource_absent",
            Self::TargetKindUnsupported => "target_kind_unsupported",
            Self::UnsupportedSection => "unsupported_section",
            Self::UnsupportedScope => "unsupported_scope",
            Self::InvalidSource => "invalid_source",
            Self::PostconditionFailed => "postcondition_failed",
            Self::AmbiguousSourceFormat => "ambiguous_source_format",
            Self::RollbackIncomplete => "rollback_incomplete",
            Self::InvalidState => "invalid_state",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::TaskBackendFailed => "task_backend_failed",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::ProfileUnsupported => "profile_unsupported",
            Self::TaskNotFound => "task_not_found",
            Self::TaskExpired => "task_expired",
            Self::Cancelled => "cancelled",
            Self::TaskProtocolFailed => "task_protocol_failed",
            Self::TaskProjectionFailed => "task_projection_failed",
            Self::ProviderFailed => "provider_failed",
            Self::InvalidResult => "invalid_result",
        }
    }

    /// Исход по умолчанию — что делать дальше, когда `detailCode` не
    /// уточняет случай.
    ///
    /// У [`Self::ProviderUnavailable`] и [`Self::TaskBackendFailed`] один код
    /// покрывает несколько исходов, и точный выбирает `detailCode`. Умолчанием
    /// у обоих стоит [`Outcome::NeedsHuman`]: неверное «нужен человек» стоит
    /// одной лишней фразы пользователю, а неверное «повторить» — холостого
    /// цикла, который на сборке базы измеряется минутами и удержанными
    /// блокировками.
    pub const fn outcome(self) -> Outcome {
        match self {
            Self::ConcurrentChange => Outcome::RetryAsIs,
            Self::SourceSelectionChanged => Outcome::RetryAsIs,
            Self::DeadlineExceeded => Outcome::RetryAsIs,
            Self::ProviderDeadline => Outcome::RetryAsIs,
            Self::DependencyUnavailable => Outcome::RetryAsIs,
            Self::TaskTransportFailed => Outcome::RetryAsIs,
            Self::TaskSessionClosed => Outcome::RetryAsIs,
            Self::BadValue => Outcome::FixCall,
            Self::NotFound => Outcome::FixCall,
            Self::SourceSetUnknown => Outcome::FixCall,
            Self::TargetNotFound => Outcome::FixCall,
            Self::ContainmentDenied => Outcome::FixCall,
            Self::InvalidCursor => Outcome::FixCall,
            Self::UnsupportedCursor => Outcome::FixCall,
            Self::UnsupportedSource => Outcome::FixCall,
            Self::UnsupportedFilter => Outcome::FixCall,
            Self::IncomparableNodes => Outcome::FixCall,
            Self::ResultTooLarge => Outcome::FixCall,
            Self::InvalidTaskId => Outcome::FixCall,
            Self::BadWaitMs => Outcome::FixCall,
            Self::BadTaskArguments => Outcome::FixCall,
            Self::RevisionMismatch => Outcome::FixCall,
            Self::StaleRevision => Outcome::FixCall,
            Self::StaleCursor => Outcome::FixCall,
            Self::ProviderLimitExceeded => Outcome::FixCall,
            Self::ResourceAbsent => Outcome::FixCall,
            Self::TargetKindUnsupported => Outcome::FixCall,
            Self::UnsupportedSection => Outcome::FixCall,
            Self::UnsupportedScope => Outcome::FixCall,
            Self::InvalidSource => Outcome::FixSource,
            Self::PostconditionFailed => Outcome::FixSource,
            Self::AmbiguousSourceFormat => Outcome::NeedsHuman,
            Self::RollbackIncomplete => Outcome::NeedsHuman,
            Self::InvalidState => Outcome::NeedsHuman,
            Self::ProviderUnavailable => Outcome::NeedsHuman,
            Self::TaskBackendFailed => Outcome::NeedsHuman,
            Self::UnsupportedOperation => Outcome::GoElsewhere,
            Self::ProfileUnsupported => Outcome::GoElsewhere,
            Self::TaskNotFound => Outcome::GoElsewhere,
            Self::TaskExpired => Outcome::GoElsewhere,
            Self::Cancelled => Outcome::DeadEnd,
            Self::TaskProtocolFailed => Outcome::DeadEnd,
            Self::TaskProjectionFailed => Outcome::DeadEnd,
            Self::ProviderFailed => Outcome::DeadEnd,
            Self::InvalidResult => Outcome::DeadEnd,
        }
    }

    /// Уточняется ли исход этого кода полем `detailCode`.
    pub const fn outcome_depends_on_detail(self) -> bool {
        matches!(self, Self::ProviderUnavailable | Self::TaskBackendFailed)
    }

    /// Весь словарь — для проверок полноты и порождения ведомостей.
    pub const ALL: [Self; 45] = [
        Self::ConcurrentChange,
        Self::SourceSelectionChanged,
        Self::DeadlineExceeded,
        Self::ProviderDeadline,
        Self::DependencyUnavailable,
        Self::TaskTransportFailed,
        Self::TaskSessionClosed,
        Self::BadValue,
        Self::NotFound,
        Self::SourceSetUnknown,
        Self::TargetNotFound,
        Self::ContainmentDenied,
        Self::InvalidCursor,
        Self::UnsupportedCursor,
        Self::UnsupportedSource,
        Self::UnsupportedFilter,
        Self::IncomparableNodes,
        Self::ResultTooLarge,
        Self::InvalidTaskId,
        Self::BadWaitMs,
        Self::BadTaskArguments,
        Self::RevisionMismatch,
        Self::StaleRevision,
        Self::StaleCursor,
        Self::ProviderLimitExceeded,
        Self::ResourceAbsent,
        Self::TargetKindUnsupported,
        Self::UnsupportedSection,
        Self::UnsupportedScope,
        Self::InvalidSource,
        Self::PostconditionFailed,
        Self::AmbiguousSourceFormat,
        Self::RollbackIncomplete,
        Self::InvalidState,
        Self::ProviderUnavailable,
        Self::TaskBackendFailed,
        Self::UnsupportedOperation,
        Self::ProfileUnsupported,
        Self::TaskNotFound,
        Self::TaskExpired,
        Self::Cancelled,
        Self::TaskProtocolFailed,
        Self::TaskProjectionFailed,
        Self::ProviderFailed,
        Self::InvalidResult,
    ];
}

impl std::fmt::Display for RefusalCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Outcome {
    /// Имена всех исходов на проводе.
    pub const ALL_WIRE_NAMES: [&'static str; 6] = [
        "retry",
        "fixCall",
        "fixSource",
        "needsHuman",
        "goElsewhere",
        "deadEnd",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_code_has_a_distinct_wire_name() {
        let names: BTreeSet<&str> = RefusalCode::ALL.iter().map(|code| code.as_str()).collect();
        assert_eq!(
            names.len(),
            RefusalCode::ALL.len(),
            "два варианта делят одно имя на проводе: словарь перестал быть словарём"
        );
    }

    #[test]
    fn wire_names_are_snake_case_and_non_empty() {
        for code in RefusalCode::ALL {
            let name = code.as_str();
            assert!(!name.is_empty(), "{code:?} без имени");
            assert!(
                name.chars().all(|character| character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || character == '_'),
                "{name} не в змеином регистре"
            );
        }
    }

    #[test]
    fn only_the_two_overloaded_codes_defer_to_detail_code() {
        let deferring: BTreeSet<&str> = RefusalCode::ALL
            .iter()
            .filter(|code| code.outcome_depends_on_detail())
            .map(|code| code.as_str())
            .collect();
        assert_eq!(
            deferring,
            BTreeSet::from(["provider_unavailable", "task_backend_failed"]),
            "список кодов, чей исход выбирает detailCode, разошёлся с разбором в проекте"
        );
    }

    #[test]
    fn every_outcome_is_reachable_from_some_code() {
        let reached: BTreeSet<&str> = RefusalCode::ALL
            .iter()
            .map(|code| code.outcome().as_str())
            .collect();
        assert_eq!(
            reached,
            BTreeSet::from(Outcome::ALL_WIRE_NAMES),
            "исход без единого кода — либо лишний, либо код для него потерян"
        );
    }

    #[test]
    fn a_met_goal_is_not_among_the_outcomes() {
        assert!(
            !Outcome::ALL_WIRE_NAMES.contains(&"alreadyMet"),
            "«цель уже достигнута» — положительный ответ, а не отказ; седьмого исхода нет"
        );
    }
}
