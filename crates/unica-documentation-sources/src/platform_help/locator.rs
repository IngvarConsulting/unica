pub const PROVIDER_ID: &str = "platform-syntax-help";

pub struct PlatformHelpLocator<'a> {
    pub version: Option<&'a str>,
    pub corpus: &'a str,
    pub path: &'a str,
}

impl<'a> PlatformHelpLocator<'a> {
    /// Parses current locators and previously issued links without a version.
    pub fn parse(document_id: &'a str) -> Result<Option<Self>, String> {
        let Some((provider, rest)) = document_id.split_once(':') else {
            return Ok(None);
        };
        if provider != PROVIDER_ID {
            return Ok(None);
        }
        let missing_parts = || format!("локатор {document_id:?} не несёт корпуса и пути");
        let (first, rest) = rest.split_once(':').ok_or_else(missing_parts)?;
        let (version, corpus, path) = match first {
            "syntax-context" | "platform-guides" => (None, first, rest),
            _ => {
                let (corpus, path) = rest.split_once(':').ok_or_else(missing_parts)?;
                (Some(first), corpus, path)
            }
        };
        if !matches!(corpus, "syntax-context" | "platform-guides") {
            return Err(format!("неизвестный корпус {corpus:?} в локаторе"));
        }
        if let Some(version) = version {
            if !version.contains('.')
                || !version.split('.').all(|component| {
                    !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
                })
            {
                return Err(format!(
                    "некорректная версия платформы {version:?} в локаторе"
                ));
            }
        }
        if path.is_empty() {
            return Err(missing_parts());
        }
        Ok(Some(Self {
            version,
            corpus,
            path,
        }))
    }
}

#[derive(Clone, Copy)]
pub(super) struct LocatorScope<'a> {
    pub version: &'a str,
    pub corpus: &'a str,
}

impl LocatorScope<'_> {
    pub fn document_id(self, path: &str) -> String {
        format!("{PROVIDER_ID}:{}:{}:{path}", self.version, self.corpus)
    }
}
