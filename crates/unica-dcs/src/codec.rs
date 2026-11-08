//! Семейный порт представления СКД. Семейство владеет моделью, намерением
//! и правилами; кодек читает, проверяет и пишет своё представление схемы.
//! Composition и общие владельцы обращаются к представлению только через
//! [`DcsCodec`], поэтому конкретный кодек не входит в их граф вызовов.
use crate::edit::DcsEdit;
use crate::facts::DcsFacts;
use serde_json::{Map, Value};
use unica_metadata::support_state::ObjectSupportData;
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyProposal, ApplySourceAuthority, ProvisionalApplyEffect,
};
use unica_source_contract::SourceSetKind;

/// Чтение, проверка и запись представления схемы компоновки данных.
pub trait DcsCodec: Send + Sync {
    /// Факты схемы из точного текста представления.
    fn read(&self, text: &str, support: ObjectSupportData) -> Result<DcsFacts, String>;

    /// Проверка представления. `label` называет проверенный вход в отчёте;
    /// кодек его не открывает.
    fn check(&self, text: &str, options: DcsCheckOptions, label: &str) -> DcsCheckReport;

    /// Применяет правку к схеме макета в предложении. Байты вне правки,
    /// неизвестный XML и метка порядка байтов сохраняются; правка без
    /// изменений ничего не пишет и не даёт события.
    fn stage(
        &self,
        staged: &mut ApplyProposal<'_>,
        authority: &dyn ApplySourceAuthority,
        edit: &DcsEdit,
        op_index: usize,
        provisional: &mut Vec<ProvisionalApplyEffect>,
    ) -> Result<(), ApplyPlanError>;

    /// Проверяет итог пакета правок: удалённый источник данных не должен
    /// оставаться в ссылках наборов.
    fn check_staged(
        &self,
        staged: &mut ApplyProposal<'_>,
        source_kind: SourceSetKind,
        edits: &[(usize, &DcsEdit)],
    ) -> Result<(), ApplyPlanError>;
}

/// Настройки проверки схемы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DcsCheckOptions {
    /// Отчёт перечисляет и пройденные проверки.
    pub detailed: bool,
    /// Проверка останавливается на этом числе ошибок.
    pub max_errors: usize,
}

impl DcsCheckOptions {
    pub fn from_args(args: &Map<String, Value>) -> Self {
        Self {
            detailed: unica_operation_contract::input::bool_arg(args, &["detailed", "Detailed"]),
            max_errors: unica_operation_contract::input::int_arg(args, &["maxErrors", "MaxErrors"])
                .unwrap_or(20)
                .max(0) as usize,
        }
    }
}

/// Итог проверки схемы: вердикт, текст отчёта и строки ошибок из него.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcsCheckReport {
    pub ok: bool,
    pub report: String,
    pub errors: Vec<String>,
}
