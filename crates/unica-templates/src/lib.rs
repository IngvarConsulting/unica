/// Формат тела макета. Семейства СКД и табличного документа применимы к
/// макету только своего формата; это единственный владелец фильтра —
/// им пользуются и словарь `can`, и планировщик, в том числе на
/// промежуточном состоянии пакета (`template.add` → `mxl.set`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateFormat {
    DataCompositionSchema,
    SpreadsheetDocument,
    Other,
}

impl TemplateFormat {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "DataCompositionSchema" => Self::DataCompositionSchema,
            "SpreadsheetDocument" => Self::SpreadsheetDocument,
            _ => Self::Other,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DataCompositionSchema => "DataCompositionSchema",
            Self::SpreadsheetDocument => "SpreadsheetDocument",
            Self::Other => "other",
        }
    }
}

pub mod address;
pub mod applicability;
pub mod contracts;
pub mod operations;

pub mod recovery;

pub mod mxl;
