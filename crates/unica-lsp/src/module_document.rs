//! Документ LSP — тело модуля.
//!
//! Редактор открывает тело модуля (`main:CommonModule.X.Body`), а службы
//! символов, диагностик и дополнения называют сам модуль
//! (`main:CommonModule.X`). Адрес документа переводится в адрес модуля и
//! обратно только здесь: служба получает этот тип из адреса документа, порты
//! продукта — вместе с вопросом, а места ответа возвращаются через него в
//! документы.

/// Сегмент, которым адрес документа отличается от адреса его модуля.
const BODY: &str = ".Body";

/// Модуль, тело которого — документ редактора.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleDocument {
    module: String,
}

impl ModuleDocument {
    /// Модуль, тело которого называет адрес документа; `None`, если адрес
    /// называет не тело модуля.
    pub fn of_document(address: &str) -> Option<Self> {
        address.strip_suffix(BODY).map(|module| Self {
            module: module.to_owned(),
        })
    }

    /// Документ тела модуля `module`: так в документ возвращается место,
    /// которое служба назвала адресом модуля.
    pub fn of_module(module: impl Into<String>) -> Self {
        Self {
            module: module.into(),
        }
    }

    /// Адрес модуля: `main:CommonModule.X`, `main:Catalog.X.Module.Object`.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Адрес документа — тела модуля.
    pub fn document(&self) -> String {
        format!("{}{BODY}", self.module)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_names_its_module_and_a_module_its_document() {
        for module in [
            "main:CommonModule.ОбщегоНазначения",
            "main:Catalog.Items.Module.Object",
            "main:Catalog.Items.Form.Item.Module.Form",
        ] {
            let document = ModuleDocument::of_module(module).document();
            let back = ModuleDocument::of_document(&document).unwrap();
            assert_eq!(back.module(), module);
            assert_eq!(back.document(), document);
        }
    }

    #[test]
    fn an_address_that_is_no_module_body_names_no_module() {
        for address in [
            "main:CommonModule.X",
            "main:CommonModule.X.Method.Ping",
            "main:CommonModule.X.Body#L4",
        ] {
            assert_eq!(ModuleDocument::of_document(address), None, "{address}");
        }
    }
}
