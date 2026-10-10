//! XML atoms shared by DCS validation and basic writers; no schema compiler.
use super::common::*;
use super::form::{
    form_is_xml_ncname, form_valid_cfg_prefixes, parse_form_decimal_contract,
    parse_form_string_contract,
};
use std::collections::BTreeMap;
pub(crate) fn resolve_type(type_str: &str) -> String {
    if type_str.is_empty() {
        return String::new();
    }
    if let Some(open) = type_str.find('(') {
        if type_str.ends_with(')') {
            let base = type_str[..open].trim();
            let params = &type_str[open + 1..type_str.len() - 1];
            if let Some(resolved) = type_synonym(base) {
                return format!("{resolved}({params})");
            }
        }
    }
    if let Some(dot_idx) = type_str.find('.') {
        let prefix = &type_str[..dot_idx];
        if let Some(resolved) = type_synonym(prefix) {
            return format!("{resolved}{}", &type_str[dot_idx..]);
        }
    }
    type_synonym(type_str).unwrap_or(type_str).to_string()
}

pub(crate) fn type_synonym(type_str: &str) -> Option<&'static str> {
    match type_str.to_lowercase().as_str() {
        "число" | "decimal" | "int" | "integer" | "number" | "num" => Some("decimal"),
        "bool" | "boolean" => Some("boolean"),
        "строка" | "str" | "string" => Some("string"),
        "булево" => Some("boolean"),
        "дата" | "date" => Some("date"),
        "датавремя" | "datetime" => Some("dateTime"),
        "время" | "time" => Some("time"),
        "стандартныйпериод" | "standardperiod" => Some("StandardPeriod"),
        "справочникссылка" => Some("CatalogRef"),
        "документссылка" => Some("DocumentRef"),
        "перечислениессылка" => Some("EnumRef"),
        "плансчетовссылка" => Some("ChartOfAccountsRef"),
        "планвидовхарактеристикссылка" => {
            Some("ChartOfCharacteristicTypesRef")
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum DcsTypeNodeKind {
    Type,
    TypeSet,
    TypeId,
}

impl DcsTypeNodeKind {
    fn tag(self) -> &'static str {
        match self {
            Self::Type => "Type",
            Self::TypeSet => "TypeSet",
            Self::TypeId => "TypeId",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DcsTypeQualifier {
    Number {
        digits: u32,
        fraction: u32,
        nonnegative: bool,
    },
    String {
        length: u32,
        fixed: bool,
    },
    Date(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DcsTypeEntry {
    kind: DcsTypeNodeKind,
    wire_name: String,
    configuration_namespace: bool,
    qualifier: Option<DcsTypeQualifier>,
}

pub(crate) fn emit_value_type(
    lines: &mut Vec<String>,
    type_spec: &str,
    indent: &str,
) -> Result<(), String> {
    let entries = parse_value_type(type_spec)?;
    emit_value_type_entries(lines, &entries, indent);
    Ok(())
}

pub(crate) fn parse_value_type(type_spec: &str) -> Result<Vec<DcsTypeEntry>, String> {
    let raw_parts = type_spec.split('|').collect::<Vec<_>>();
    if raw_parts.is_empty() || raw_parts.iter().any(|part| part.trim().is_empty()) {
        return Err(format!(
            "DCS type '{type_spec}' is not valid for 8.3.27: composite type contains an empty item"
        ));
    }

    let entries = raw_parts
        .iter()
        .map(|part| parse_type_entry(part.trim()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("DCS type '{type_spec}' is not valid for 8.3.27: {error}"))?;

    let mut seen = BTreeMap::<(DcsTypeNodeKind, String), &str>::new();
    for (raw, entry) in raw_parts.iter().zip(&entries) {
        let key = (entry.kind, entry.wire_name.clone());
        if let Some(previous) = seen.insert(key, raw.trim()) {
            return Err(format!(
                "DCS type '{type_spec}' is not valid for 8.3.27: duplicate platform type '{previous}' and '{}' both map to v8:{} {}",
                raw.trim(),
                entry.kind.tag(),
                entry.wire_name
            ));
        }
    }

    Ok(entries)
}

pub(crate) fn parse_type_entry(type_name: &str) -> Result<DcsTypeEntry, String> {
    let normalized = resolve_type(type_name);
    if normalized == "boolean" {
        return Ok(dcs_type_entry(DcsTypeNodeKind::Type, "xs:boolean", false));
    }
    if normalized == "StandardPeriod" {
        return Ok(dcs_type_entry(
            DcsTypeNodeKind::Type,
            "v8:StandardPeriod",
            false,
        ));
    }
    if normalized == "string" {
        return Ok(dcs_type_qualified_entry(
            "xs:string",
            DcsTypeQualifier::String {
                length: 0,
                fixed: false,
            },
        ));
    }
    if normalized.starts_with("string(") {
        let (length, fixed) = parse_form_string_contract(&normalized).ok_or_else(|| {
            format!(
                "type '{type_name}' must be string(integer length 0..=1024[,fixed|variable]); fixed requires length > 0"
            )
        })?;
        return Ok(dcs_type_qualified_entry(
            "xs:string",
            DcsTypeQualifier::String { length, fixed },
        ));
    }
    if normalized == "decimal" {
        return Ok(dcs_type_qualified_entry(
            "xs:decimal",
            DcsTypeQualifier::Number {
                digits: 10,
                fraction: 2,
                nonnegative: false,
            },
        ));
    }
    if normalized.starts_with("decimal(") {
        let (digits, fraction, nonnegative) =
            parse_decimal_contract(&normalized).ok_or_else(|| {
                format!(
                    "type '{type_name}' must be decimal(integer digits 0..=38[, integer fraction 0..=digits][,nonneg])"
                )
            })?;
        return Ok(dcs_type_qualified_entry(
            "xs:decimal",
            DcsTypeQualifier::Number {
                digits,
                fraction,
                nonnegative,
            },
        ));
    }
    if matches!(normalized.as_str(), "date" | "dateTime" | "time") {
        let fractions = match normalized.as_str() {
            "date" => "Date",
            "dateTime" => "DateTime",
            "time" => "Time",
            _ => unreachable!(),
        };
        return Ok(dcs_type_qualified_entry(
            "xs:dateTime",
            DcsTypeQualifier::Date(fractions),
        ));
    }
    if let Some(type_id) = normalized.strip_prefix("typeid:") {
        if !is_valid_uuid(type_id) {
            return Err(format!("type '{type_name}' has an invalid TypeId UUID"));
        }
        return Ok(dcs_type_entry(
            DcsTypeNodeKind::TypeId,
            &type_id.to_ascii_lowercase(),
            false,
        ));
    }
    if normalized.starts_with("DefinedType.") {
        validate_configuration_type(type_name, &normalized)?;
        return Err(format!(
            "type '{type_name}' is not supported by the fixed 8.3.27 DCS contract: platform 8.3.27 removes DefinedType.* from valueType during round-trip; use the defined type's expanded constituent types"
        ));
    }
    if normalized.starts_with("Characteristic.") {
        validate_configuration_type(type_name, &normalized)?;
        return Ok(dcs_type_entry(
            DcsTypeNodeKind::TypeSet,
            &format!("d5p1:{normalized}"),
            true,
        ));
    }
    if let Some((prefix, _)) = normalized.split_once('.') {
        if !form_valid_cfg_prefixes().contains(&prefix) {
            return Err(format!(
                "type '{type_name}' has unknown configuration type prefix '{prefix}'"
            ));
        }
        validate_configuration_type(type_name, &normalized)?;
        return Ok(dcs_type_entry(
            DcsTypeNodeKind::Type,
            &format!("d5p1:{normalized}"),
            true,
        ));
    }
    // In a field type descriptor a bare XML name denotes a configuration
    // TypeSet. Its existence can only be checked against a concrete configuration.
    if form_is_xml_ncname(&normalized) {
        return Ok(dcs_type_entry(
            DcsTypeNodeKind::TypeSet,
            &format!("d5p1:{normalized}"),
            true,
        ));
    }
    Err(format!(
        "type '{type_name}' is not supported by the fixed 8.3.27 DCS type contract"
    ))
}

fn parse_decimal_contract(value: &str) -> Option<(u32, u32, bool)> {
    let rest = value.strip_prefix("decimal(")?.strip_suffix(')')?;
    let parts = rest.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() == 1 {
        let digits = parts[0]
            .parse::<u32>()
            .ok()
            .filter(|digits| *digits <= 38)?;
        return Some((digits, 0, false));
    }
    parse_form_decimal_contract(value)
}

fn validate_configuration_type(raw: &str, normalized: &str) -> Result<(), String> {
    let invalid_name = normalized
        .split_once('.')
        .is_none_or(|(_, name)| name.trim().is_empty() || name.contains('.'));
    if invalid_name || !form_is_xml_ncname(normalized) {
        return Err(format!(
            "type '{raw}' has an invalid or empty configuration type name"
        ));
    }
    Ok(())
}

fn dcs_type_entry(
    kind: DcsTypeNodeKind,
    wire_name: &str,
    configuration_namespace: bool,
) -> DcsTypeEntry {
    DcsTypeEntry {
        kind,
        wire_name: wire_name.to_string(),
        configuration_namespace,
        qualifier: None,
    }
}

fn dcs_type_qualified_entry(wire_name: &str, qualifier: DcsTypeQualifier) -> DcsTypeEntry {
    DcsTypeEntry {
        qualifier: Some(qualifier),
        ..dcs_type_entry(DcsTypeNodeKind::Type, wire_name, false)
    }
}

fn emit_value_type_entries(lines: &mut Vec<String>, entries: &[DcsTypeEntry], indent: &str) {
    for kind in [
        DcsTypeNodeKind::Type,
        DcsTypeNodeKind::TypeSet,
        DcsTypeNodeKind::TypeId,
    ] {
        for entry in entries.iter().filter(|entry| entry.kind == kind) {
            let tag = entry.kind.tag();
            if entry.configuration_namespace {
                lines.push(format!(
                    "{indent}<v8:{tag} xmlns:d5p1=\"http://v8.1c.ru/8.1/data/enterprise/current-config\">{}</v8:{tag}>",
                    escape_xml(&entry.wire_name)
                ));
            } else {
                lines.push(format!(
                    "{indent}<v8:{tag}>{}</v8:{tag}>",
                    escape_xml(&entry.wire_name)
                ));
            }
        }
    }
    for qualifier_rank in [0_u8, 1, 2] {
        for qualifier in entries.iter().filter_map(|entry| entry.qualifier) {
            if dcs_type_qualifier_rank(qualifier) == qualifier_rank {
                emit_type_qualifier(lines, qualifier, indent);
            }
        }
    }
}

fn dcs_type_qualifier_rank(qualifier: DcsTypeQualifier) -> u8 {
    match qualifier {
        DcsTypeQualifier::Number { .. } => 0,
        DcsTypeQualifier::String { .. } => 1,
        DcsTypeQualifier::Date(_) => 2,
    }
}

fn emit_type_qualifier(lines: &mut Vec<String>, qualifier: DcsTypeQualifier, indent: &str) {
    match qualifier {
        DcsTypeQualifier::Number {
            digits,
            fraction,
            nonnegative,
        } => {
            lines.push(format!("{indent}<v8:NumberQualifiers>"));
            lines.push(format!("{indent}\t<v8:Digits>{digits}</v8:Digits>"));
            lines.push(format!(
                "{indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
            ));
            lines.push(format!(
                "{indent}\t<v8:AllowedSign>{}</v8:AllowedSign>",
                if nonnegative { "Nonnegative" } else { "Any" }
            ));
            lines.push(format!("{indent}</v8:NumberQualifiers>"));
        }
        DcsTypeQualifier::String { length, fixed } => {
            lines.push(format!("{indent}<v8:StringQualifiers>"));
            lines.push(format!("{indent}\t<v8:Length>{length}</v8:Length>"));
            lines.push(format!(
                "{indent}\t<v8:AllowedLength>{}</v8:AllowedLength>",
                if fixed { "Fixed" } else { "Variable" }
            ));
            lines.push(format!("{indent}</v8:StringQualifiers>"));
        }
        DcsTypeQualifier::Date(fractions) => {
            lines.push(format!("{indent}<v8:DateQualifiers>"));
            lines.push(format!(
                "{indent}\t<v8:DateFractions>{fractions}</v8:DateFractions>"
            ));
            lines.push(format!("{indent}</v8:DateQualifiers>"));
        }
    }
}

pub(crate) fn is_valid_xs_decimal(value: &str) -> bool {
    let value = value.strip_prefix(['+', '-']).unwrap_or(value);
    let Some((integer, fraction)) = value.split_once('.') else {
        return !value.is_empty() && value.chars().all(|character| character.is_ascii_digit());
    };
    !value[integer.len() + 1..].contains('.')
        && (!integer.is_empty() || !fraction.is_empty())
        && integer.chars().all(|character| character.is_ascii_digit())
        && fraction.chars().all(|character| character.is_ascii_digit())
}

pub(crate) fn is_date_time_literal(value: &str) -> bool {
    let Some((date, time_and_zone)) = value.split_once('T') else {
        return false;
    };
    if time_and_zone.contains('T') {
        return false;
    }
    let mut date_parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) = (
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
    ) else {
        return false;
    };
    if year.len() < 4
        || !year.chars().all(|character| character.is_ascii_digit())
        || year.chars().all(|character| character == '0')
    {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<u32>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > max_day {
        return false;
    }

    let (time, zone) = if let Some(time) = time_and_zone.strip_suffix('Z') {
        (time, Some("Z"))
    } else if let Some(index) = time_and_zone
        .char_indices()
        .skip(1)
        .find_map(|(index, character)| matches!(character, '+' | '-').then_some(index))
    {
        (&time_and_zone[..index], Some(&time_and_zone[index..]))
    } else {
        (time_and_zone, None)
    };
    let mut time_parts = time.split(':');
    let (Some(hour), Some(minute), Some(second), None) = (
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
    ) else {
        return false;
    };
    let (second, fraction) = second
        .split_once('.')
        .map_or((second, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    if fraction.is_some_and(|fraction| {
        fraction.is_empty() || !fraction.chars().all(|character| character.is_ascii_digit())
    }) {
        return false;
    }
    let (Ok(hour), Ok(minute), Ok(second)) = (
        hour.parse::<u32>(),
        minute.parse::<u32>(),
        second.parse::<u32>(),
    ) else {
        return false;
    };
    if minute > 59 || second > 59 || hour > 24 {
        return false;
    }
    if hour == 24
        && (minute != 0
            || second != 0
            || fraction.is_some_and(|fraction| fraction.chars().any(|digit| digit != '0')))
    {
        return false;
    }
    if let Some(zone) = zone.filter(|zone| *zone != "Z") {
        let zone = &zone[1..];
        let Some((hours, minutes)) = zone.split_once(':') else {
            return false;
        };
        let (Ok(hours), Ok(minutes)) = (hours.parse::<u32>(), minutes.parse::<u32>()) else {
            return false;
        };
        if hours > 14 || minutes > 59 || (hours == 14 && minutes != 0) {
            return false;
        }
    }
    true
}

pub(crate) fn validate_field_role_value(key: &str, value: &str) -> Result<(), String> {
    let valid = match key {
        "periodNumber" => {
            let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
            !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
        }
        "periodType" => matches!(value, "Main" | "Specify" | "Additional"),
        "dimension" | "account" | "balance" | "ignoreNullValues" | "required"
        | "dimensionAttribute" => matches!(value, "true" | "false" | "0" | "1"),
        "balanceType" => matches!(value, "None" | "OpeningBalance" | "ClosingBalance"),
        "accountingBalanceType" => matches!(value, "None" | "Debit" | "Credit"),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "Role value '{value}' is invalid for '{key}' in the fixed DCS 8.3.27 XSD contract"
        ))
    }
}

pub(crate) fn xsi_type_matches(
    node: roxmltree::Node<'_, '_>,
    expected_namespace: &str,
    expected_local_name: &str,
) -> bool {
    let Some(value) = node.attribute((super::dcs::XML_SCHEMA_INSTANCE_NS, "type")) else {
        return false;
    };
    let Some((prefix, local_name)) = value.split_once(':') else {
        return value == expected_local_name
            && node.lookup_namespace_uri(None) == Some(expected_namespace);
    };
    local_name == expected_local_name
        && !prefix.contains(':')
        && node.lookup_namespace_uri(Some(prefix)) == Some(expected_namespace)
}

pub(crate) fn is_standard_period_variant(value: &str) -> bool {
    matches!(
        value,
        "Custom"
            | "Today"
            | "ThisWeek"
            | "ThisTenDays"
            | "ThisMonth"
            | "ThisQuarter"
            | "ThisHalfYear"
            | "ThisYear"
            | "FromBeginningOfThisWeek"
            | "FromBeginningOfThisTenDays"
            | "FromBeginningOfThisMonth"
            | "FromBeginningOfThisQuarter"
            | "FromBeginningOfThisHalfYear"
            | "FromBeginningOfThisYear"
            | "Yesterday"
            | "LastWeek"
            | "LastTenDays"
            | "LastMonth"
            | "LastQuarter"
            | "LastHalfYear"
            | "LastYear"
            | "LastWeekTillSameWeekDay"
            | "LastTenDaysTillSameDayNumber"
            | "LastMonthTillSameDate"
            | "LastQuarterTillSameDate"
            | "LastHalfYearTillSameDate"
            | "LastYearTillSameDate"
            | "Tomorrow"
            | "NextWeek"
            | "NextTenDays"
            | "NextMonth"
            | "NextQuarter"
            | "NextHalfYear"
            | "NextYear"
            | "NextWeekTillSameWeekDay"
            | "NextTenDaysTillSameDayNumber"
            | "NextMonthTillSameDate"
            | "NextQuarterTillSameDate"
            | "NextHalfYearTillSameDate"
            | "NextYearTillSameDate"
            | "TillEndOfThisWeek"
            | "TillEndOfThisTenDays"
            | "TillEndOfThisMonth"
            | "TillEndOfThisQuarter"
            | "TillEndOfThisHalfYear"
            | "TillEndOfThisYear"
            | "Last7Days"
            | "Next7Days"
            | "Month"
    )
}
