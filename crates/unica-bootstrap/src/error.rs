use std::fmt;

/// Чем кончилась попытка. Вызывающий отличает недоступную сеть от подменённых
/// байтов по коду выхода, не читая текста: убитая посреди старта сессия текста
/// не увидит вовсе.
///
/// Номера взяты из `sysexits.h`, потому что своя нумерация здесь ничего не
/// добавляет, а чужая уже знакома оболочкам и хостам.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    /// Канал не отдал байты: соединение, имя, обрыв.
    Network,
    /// Байты шли, но не кончились в отведённое время.
    Timeout,
    /// Диск: нет места, нет прав, нет каталога.
    Disk,
    /// Приехало не то, что обещано манифестом.
    Checksum,
    /// Цель не обслуживается или манифест не годится.
    Configuration,
    /// Всё остальное — дефект самого bootstrap.
    Internal,
}

impl Failure {
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Network => 69,
            Self::Disk => 74,
            Self::Timeout => 75,
            Self::Checksum => 76,
            Self::Configuration => 78,
            Self::Internal => 70,
        }
    }

    pub const fn reason(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::Disk => "disk",
            Self::Checksum => "checksum",
            Self::Configuration => "configuration",
            Self::Internal => "internal",
        }
    }

    /// Что делать тому, кто это прочитал.
    pub const fn cure(self) -> &'static str {
        match self {
            Self::Network => {
                "run again — what already arrived is kept and the download resumes"
            }
            Self::Timeout => {
                "run again on a faster channel — what already arrived is kept and the download resumes"
            }
            Self::Disk => "free space in the runtime cache, or point UNICA_ARTIFACT_CACHE elsewhere",
            Self::Checksum => {
                "the archive does not match the manifest: reinstall the plugin, and report it if it repeats"
            }
            Self::Configuration => {
                "the installed plugin does not match this host, or is incomplete: reinstall Unica for a supported target"
            }
            Self::Internal => "report it: the bootstrap reached a state it does not describe",
        }
    }
}

#[derive(Debug)]
pub struct BootstrapError {
    message: String,
    failure: Failure,
}

impl BootstrapError {
    /// Неклассифицированный отказ. Остаётся для мест, где разбор ничего не
    /// добавляет: код выхода у них один — дефект bootstrap.
    pub fn new(message: impl Into<String>) -> Self {
        Self::of(Failure::Internal, message)
    }

    pub fn of(failure: Failure, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            failure,
        }
    }

    pub const fn failure(&self) -> Failure {
        self.failure
    }

    pub const fn exit_code(&self) -> u8 {
        self.failure.exit_code()
    }

    /// Причина, место и лечение. Место называет само сообщение: каждое из них
    /// несёт адрес или путь, на котором споткнулось.
    pub fn diagnosis(&self) -> String {
        format!(
            "{}\n  reason: {}\n  cure: {}",
            self.message,
            self.failure.reason(),
            self.failure.cure()
        )
    }
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BootstrapError {}

impl From<std::io::Error> for BootstrapError {
    fn from(error: std::io::Error) -> Self {
        Self::of(Failure::Disk, error.to_string())
    }
}

impl From<serde_json::Error> for BootstrapError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, BootstrapError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_failure_leaves_its_own_exit_code() {
        // Коды из sysexits: вызывающий отличает недоступную сеть от подменённых
        // байтов, не читая текста.
        assert_eq!(Failure::Network.exit_code(), 69);
        assert_eq!(Failure::Timeout.exit_code(), 75);
        assert_eq!(Failure::Disk.exit_code(), 74);
        assert_eq!(Failure::Checksum.exit_code(), 76);
        assert_eq!(Failure::Configuration.exit_code(), 78);
        assert_eq!(Failure::Internal.exit_code(), 70);
    }

    #[test]
    fn the_launcher_codes_stay_free() {
        // 64, 66 и 78 принадлежат `launch.sh`; из них bootstrap занимает только
        // 78, и с тем же смыслом — цель, которую не обслуживают.
        let taken = [
            Failure::Network,
            Failure::Timeout,
            Failure::Disk,
            Failure::Checksum,
            Failure::Internal,
        ]
        .map(Failure::exit_code);
        assert!(!taken.contains(&64));
        assert!(!taken.contains(&66));
        assert!(!taken.contains(&78));
    }

    #[test]
    fn a_disk_error_arrives_classified_without_being_told() {
        let error: BootstrapError = std::io::Error::other("no space left").into();

        assert_eq!(error.failure(), Failure::Disk);
    }

    #[test]
    fn a_diagnosis_names_the_reason_the_place_and_the_cure() {
        let error = BootstrapError::of(
            Failure::Network,
            "failed to download runtime asset https://example.com/unica.tar.gz: connection reset",
        );

        let diagnosis = error.diagnosis();

        assert!(
            diagnosis.contains("https://example.com/unica.tar.gz"),
            "место названо: {diagnosis}"
        );
        assert!(
            diagnosis.contains("reason: network"),
            "причина: {diagnosis}"
        );
        assert!(diagnosis.contains("cure:"), "лечение: {diagnosis}");
        assert!(
            diagnosis.contains("resumes"),
            "лечение сети — повторить запуск: {diagnosis}"
        );
    }
}
