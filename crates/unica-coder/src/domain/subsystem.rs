use serde::Serialize;
use std::fmt;

pub const SUBSYSTEM_ADDRESS_MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SubsystemAddress(String);

impl SubsystemAddress {
    pub fn parse(raw: &str) -> Result<Self, SubsystemAddressError> {
        Self::from_names(raw.split('.'))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn from_names<'a>(
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, SubsystemAddressError> {
        let names = names.into_iter().collect::<Vec<_>>();
        if names.is_empty() || names.len() > SUBSYSTEM_ADDRESS_MAX_DEPTH {
            return Err(SubsystemAddressError::new(format!(
                "subsystem address must contain from 1 to {SUBSYSTEM_ADDRESS_MAX_DEPTH} names"
            )));
        }
        for name in &names {
            if !is_1c_identifier(name) {
                return Err(SubsystemAddressError::new(format!(
                    "subsystem address contains invalid name `{name}`"
                )));
            }
        }
        Ok(Self(names.join(".")))
    }
}

impl fmt::Display for SubsystemAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsystemAddressError {
    message: String,
}

impl SubsystemAddressError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SubsystemAddressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SubsystemAddressError {}

fn is_1c_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_1c_identifier_start(first) && chars.all(is_1c_identifier_part)
}

fn is_1c_identifier_start(ch: char) -> bool {
    ch == '_'
        || ch.is_ascii_alphabetic()
        || ('А'..='Я').contains(&ch)
        || ('а'..='я').contains(&ch)
        || ch == 'Ё'
        || ch == 'ё'
}

fn is_1c_identifier_part(ch: char) -> bool {
    is_1c_identifier_start(ch) || ch.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bsl_address_serializes_without_platform_kind_tokens() {
        let address = SubsystemAddress::parse("СтандартныеПодсистемы.Обсуждения").unwrap();

        assert_eq!(address.as_str(), "СтандартныеПодсистемы.Обсуждения");
        assert_eq!(
            serde_json::to_value(address).unwrap(),
            serde_json::json!("СтандартныеПодсистемы.Обсуждения")
        );
    }

    #[test]
    fn address_preserves_the_spelling_of_application_names() {
        let address = SubsystemAddress::parse("Sales.eCommerce").unwrap();

        assert_eq!(address.as_str(), "Sales.eCommerce");
    }

    #[test]
    fn address_rejects_empty_and_non_identifier_segments() {
        for raw in ["", ".A", "A.", "A..B", "1Root", "A.With space", "A.A-B"] {
            assert!(
                SubsystemAddress::parse(raw).is_err(),
                "invalid subsystem address was accepted: {raw}"
            );
        }
    }

    #[test]
    fn depth_budget_counts_subsystem_names_not_a_kind_token() {
        let maximum = "A.B.C.D.E.F.G.H";
        assert_eq!(SubsystemAddress::parse(maximum).unwrap().as_str(), maximum);
        assert!(SubsystemAddress::parse("A.B.C.D.E.F.G.H.I").is_err());
    }

    #[test]
    fn registered_names_build_the_same_public_address() {
        let address = SubsystemAddress::from_names(["СтандартныеПодсистемы", "Обсуждения"])
            .unwrap();

        assert_eq!(address.as_str(), "СтандартныеПодсистемы.Обсуждения");
    }
}
