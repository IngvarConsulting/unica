/// Адрес подсистемы в плоском диалекте БСП: `СтандартныеПодсистемы.Обсуждения`.
///
/// Вид не называется ни в начале, ни между уровнями. Это тот же диалект, что
/// принимает `ПодсистемаСуществует`, поэтому адрес читается разработчиком без
/// перевода.
///
/// Отдельный тип, а не расширение [`MetadataAddress`](super::source_target::MetadataAddress):
/// там вид цели выводится из чётности числа сегментов, поэтому вложенная
/// подсистема была бы прочитана как модуль. Платформенные формы
/// (`Subsystem.A.Subsystem.B` в `Rights.xml`, в `Content/xr:Item` и в объектной
/// модели BSL) остаются на границе XML и превращаются в этот тип при разборе.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SubsystemAddress(String);

impl SubsystemAddress {
    /// Собирает адрес из пути вложенности. Пустое имя или имя с точкой сделали
    /// бы адрес неоднозначным, поэтому отвергаются.
    pub(crate) fn from_names<S: AsRef<str>>(names: &[S]) -> Result<Self, String> {
        if names.is_empty() {
            return Err("subsystem address has no names".to_string());
        }
        for name in names {
            let name = name.as_ref();
            if name.is_empty() {
                return Err("subsystem address has an empty name".to_string());
            }
            if name.contains('.') {
                return Err(format!("subsystem name `{name}` contains a separator"));
            }
        }
        Ok(Self(
            names
                .iter()
                .map(|name| name.as_ref())
                .collect::<Vec<_>>()
                .join("."),
        ))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_address_reads_as_the_bsp_dialect() {
        let address =
            SubsystemAddress::from_names(&["СтандартныеПодсистемы", "Обсуждения"]).unwrap();

        assert_eq!(address.as_str(), "СтандартныеПодсистемы.Обсуждения");
    }

    #[test]
    fn an_ambiguous_name_is_refused() {
        assert!(SubsystemAddress::from_names::<&str>(&[]).is_err());
        assert!(SubsystemAddress::from_names(&["A", ""]).is_err());
        assert!(SubsystemAddress::from_names(&["A.B"]).is_err());
    }
}
