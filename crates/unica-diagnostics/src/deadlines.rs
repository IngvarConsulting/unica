// resource-limit: diagnostics.analyze-timeout-min
// class: deadline
// unit: s
// scope: request
// value: 30
// resource: Нижняя граница явного срока анализа диагностики (timeoutSeconds).
// status: unproven
// origin: коммит 0e5385ca5 (allow bounded diagnostics analysis timeout)
// problem: неизвестно
// on-reach: явный аргумент вне диапазона отклоняется OutOfRange «must be between 30 and 3600»; срок из настроек прижимается к границе
// decision: закреплено правилом
// decision-date: 2026-10-21
// decision-source: arch/rules/analysis/diagnostics-timeout-overlay.md
pub const DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS: u64 = 30;

// resource-limit: diagnostics.analyze-timeout-max
// class: deadline
// unit: s
// scope: request
// value: 3600
// resource: Верхняя граница явного срока анализа диагностики (timeoutSeconds).
// status: unproven
// origin: коммит 0e5385ca5 (allow bounded diagnostics analysis timeout)
// problem: неизвестно
// on-reach: явный аргумент вне диапазона отклоняется OutOfRange «must be between 30 and 3600»; срок из настроек прижимается к границе
// decision: закреплено правилом
// decision-date: 2026-10-21
// decision-source: arch/rules/analysis/diagnostics-timeout-overlay.md
pub const DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS: u64 = 3600;
