//! Existing shared scalar type contracts and configuration value-type spellings.

pub fn valid_cfg_prefixes() -> &'static [&'static str] {
    &[
        "AccountingRegisterRecordSet",
        "AccumulationRegisterRecordSet",
        "BusinessProcessObject",
        "BusinessProcessRef",
        "CalculationRegisterRecordSet",
        "CatalogObject",
        "CatalogRef",
        "ChartOfAccountsObject",
        "ChartOfAccountsRef",
        "ChartOfCalculationTypesObject",
        "ChartOfCalculationTypesRef",
        "ChartOfCharacteristicTypesObject",
        "ChartOfCharacteristicTypesRef",
        "ConstantsSet",
        "DataProcessorObject",
        "DocumentObject",
        "DocumentRef",
        "DynamicList",
        "EnumRef",
        "ExchangePlanObject",
        "ExchangePlanRef",
        "ExternalDataProcessorObject",
        "ExternalReportObject",
        "InformationRegisterRecordManager",
        "InformationRegisterRecordSet",
        "ReportObject",
        "TaskObject",
        "TaskRef",
    ]
}

pub fn parse_string_contract(value: &str) -> Option<(u32, bool)> {
    let rest = value.strip_prefix("string(")?.strip_suffix(')')?;
    let parts = rest.split(',').map(str::trim).collect::<Vec<_>>();
    if !matches!(parts.len(), 1 | 2) || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let length = parts[0]
        .parse::<u32>()
        .ok()
        .filter(|length| *length <= 1024)?;
    let fixed = match parts.get(1).map(|value| value.to_ascii_lowercase()) {
        None => false,
        Some(value) if value == "variable" => false,
        Some(value) if value == "fixed" && length > 0 => true,
        _ => return None,
    };
    Some((length, fixed))
}

pub fn parse_decimal_contract(value: &str) -> Option<(u32, u32, bool)> {
    let rest = value.strip_prefix("decimal(")?.strip_suffix(')')?;
    let parts = rest.split(',').map(str::trim).collect::<Vec<_>>();
    if !matches!(parts.len(), 2 | 3)
        || parts.iter().any(|part| part.is_empty())
        || (parts.len() == 3 && parts[2] != "nonneg")
    {
        return None;
    }
    let digits = parts[0].parse::<u32>().ok()?;
    let fraction = parts[1].parse::<u32>().ok()?;
    if digits > 38 || fraction > digits {
        return None;
    }
    Some((digits, fraction, parts.len() == 3))
}
