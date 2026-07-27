use crate::domain::{navigation::Authorability, navigation_limits::MAX_NAVIGATION_NESTING_DEPTH};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

const MAX_PARENT_CONFIGURATIONS_BYTES: u64 = 1024 * 1024;
const MAX_VENDOR_COUNT: usize = 8;
const MAX_RULES_PER_VENDOR: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupportParseErrorKind {
    Io,
    InputTooLarge,
    NotRegularFile,
    InvalidUtf8,
    InvalidEscape,
    UnterminatedString,
    UnexpectedToken,
    Truncated,
    TrailingData,
    UnsupportedVersion,
    UnsupportedLayout,
    UnknownCode,
    InvalidUuid,
    DuplicateRule,
    ConflictingEvidence,
    NestingLimitExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportParseError {
    pub(crate) kind: SupportParseErrorKind,
    pub(crate) offset: Option<usize>,
    pub(crate) context: &'static str,
}

impl SupportParseError {
    fn new(kind: SupportParseErrorKind, offset: usize, context: &'static str) -> Self {
        Self {
            kind,
            offset: Some(offset),
            context,
        }
    }

    fn unknown(kind: SupportParseErrorKind, context: &'static str) -> Self {
        Self {
            kind,
            offset: None,
            context,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SupportSourceState {
    Absent,
    Removed,
    Parsed,
    Unreadable { error: SupportParseError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupportRule {
    Locked,
    Editable,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectiveSupportRule {
    Absent,
    Removed,
    Editable,
    Locked,
    ConfigurationReadOnly,
    UnknownReadOnly,
    Unreadable,
}

impl EffectiveSupportRule {
    pub(crate) fn authorability(self) -> Authorability {
        match self {
            Self::Absent | Self::Removed | Self::Editable => Authorability::Authorable,
            Self::Locked => Authorability::SupportLocked,
            Self::ConfigurationReadOnly => Authorability::ConfigurationReadOnly,
            Self::UnknownReadOnly => Authorability::UnknownReadOnly,
            Self::Unreadable => Authorability::UnknownSupportState,
        }
    }
}

impl SupportRule {
    pub(crate) fn flag(self) -> u8 {
        match self {
            Self::Locked => 0,
            Self::Editable => 1,
            Self::Removed => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportVendor {
    pub(crate) provider_uuid: String,
    pub(crate) vendor_flag: bool,
    pub(crate) vendor_configuration_uuid: String,
    pub(crate) version: String,
    pub(crate) vendor: String,
    pub(crate) name: String,
    pub(crate) configuration_rule: Option<(String, SupportRule)>,
    pub(crate) object_rules: BTreeMap<String, SupportRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportFacts {
    pub(crate) source: SupportSourceState,
    pub(crate) object_rules: BTreeMap<String, SupportRule>,
    global_editing_enabled: Option<bool>,
    configuration_rule: Option<(String, SupportRule)>,
    vendors: Vec<SupportVendor>,
    multi_vendor_semantics_unproven: bool,
}

impl SupportFacts {
    pub(crate) fn effective_rule_for(&self, object: &str) -> EffectiveSupportRule {
        match self.source {
            SupportSourceState::Absent => return EffectiveSupportRule::Absent,
            SupportSourceState::Removed => return EffectiveSupportRule::Removed,
            SupportSourceState::Unreadable { .. } => return EffectiveSupportRule::Unreadable,
            SupportSourceState::Parsed => {}
        }
        if self.multi_vendor_semantics_unproven {
            return EffectiveSupportRule::UnknownReadOnly;
        }
        if self.global_editing_enabled == Some(false) {
            return EffectiveSupportRule::ConfigurationReadOnly;
        }
        let object = object.to_ascii_lowercase();
        let configuration = self.configuration_rule.as_ref().map(|(_, rule)| *rule);
        let object_rule = self.object_rules.get(&object).copied().or_else(|| {
            self.configuration_rule
                .as_ref()
                .and_then(|(uuid, rule)| (uuid == &object).then_some(*rule))
        });
        if matches!(configuration, Some(SupportRule::Locked))
            || matches!(object_rule, Some(SupportRule::Locked))
        {
            return EffectiveSupportRule::Locked;
        }
        match object_rule {
            Some(SupportRule::Removed) => EffectiveSupportRule::Removed,
            Some(SupportRule::Editable) => EffectiveSupportRule::Editable,
            Some(SupportRule::Locked) => unreachable!("locked rules return before this match"),
            None => EffectiveSupportRule::Absent,
        }
    }

    pub(crate) fn authorability_for(&self, object: &str) -> Authorability {
        if matches!(
            &self.source,
            SupportSourceState::Unreadable { error }
                if error.kind == SupportParseErrorKind::InputTooLarge
        ) {
            return Authorability::UnknownReadOnly;
        }
        self.effective_rule_for(object).authorability()
    }

    pub(crate) fn parse_error(&self) -> Option<&SupportParseError> {
        match &self.source {
            SupportSourceState::Unreadable { error } => Some(error),
            _ => None,
        }
    }

    pub(crate) fn global_editing_enabled(&self) -> Option<bool> {
        self.global_editing_enabled
    }

    pub(crate) fn vendors(&self) -> &[SupportVendor] {
        &self.vendors
    }

    pub(crate) fn rule_counts(&self) -> [usize; 3] {
        let mut counts = [0; 3];
        if let Some((_, rule)) = self.configuration_rule.as_ref() {
            counts[rule.flag() as usize] += 1;
        }
        for rule in self.object_rules.values() {
            counts[rule.flag() as usize] += 1;
        }
        counts
    }
}

pub(crate) fn read_support_facts(bin_path: &Path) -> SupportFacts {
    let metadata = match fs::symlink_metadata(bin_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return absent(),
        Err(_) => {
            return unreadable(SupportParseError::unknown(
                SupportParseErrorKind::Io,
                "metadata",
            ))
        }
    };
    if !metadata.file_type().is_file() {
        return unreadable(SupportParseError::unknown(
            SupportParseErrorKind::NotRegularFile,
            "support file",
        ));
    }
    if metadata.len() > MAX_PARENT_CONFIGURATIONS_BYTES {
        return unreadable(SupportParseError::unknown(
            SupportParseErrorKind::InputTooLarge,
            "support file",
        ));
    }
    let mut file = match fs::File::open(bin_path) {
        Ok(file) => file,
        Err(_) => {
            return unreadable(SupportParseError::unknown(
                SupportParseErrorKind::Io,
                "open",
            ))
        }
    };
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    if file
        .by_ref()
        .take(MAX_PARENT_CONFIGURATIONS_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return unreadable(SupportParseError::unknown(
            SupportParseErrorKind::Io,
            "read",
        ));
    }
    if bytes.len() as u64 > MAX_PARENT_CONFIGURATIONS_BYTES {
        return unreadable(SupportParseError::unknown(
            SupportParseErrorKind::InputTooLarge,
            "support file",
        ));
    }
    parse_parent_configurations(&bytes)
}

/// Parse support evidence already captured by a Platform XML provider.  An
/// absent file in an immutable snapshot is explicit evidence that the source
/// is not on support, not an invitation to inspect the live filesystem again.
pub(crate) fn read_support_facts_bytes(bytes: Option<&[u8]>) -> SupportFacts {
    match bytes {
        Some(bytes) if bytes.len() <= 1024 * 1024 => parse_parent_configurations(bytes),
        Some(_) => unreadable(SupportParseError::new(
            SupportParseErrorKind::InputTooLarge,
            1024 * 1024,
            "ParentConfigurations.bin",
        )),
        None => absent(),
    }
}

pub(crate) fn read_support_facts_bytes_for_configuration(
    bytes: Option<&[u8]>,
    configuration_uuid: &str,
) -> SupportFacts {
    match bytes {
        Some(bytes) if bytes.len() <= 1024 * 1024 => {
            parse_parent_configurations_for_configuration(bytes, configuration_uuid)
        }
        Some(_) => unreadable(SupportParseError::new(
            SupportParseErrorKind::InputTooLarge,
            1024 * 1024,
            "ParentConfigurations.bin",
        )),
        None => absent(),
    }
}

pub(crate) fn unreadable_configuration_evidence() -> SupportFacts {
    unreadable(SupportParseError::unknown(
        SupportParseErrorKind::ConflictingEvidence,
        "Configuration.xml",
    ))
}

pub(crate) fn parse_parent_configurations(input: &[u8]) -> SupportFacts {
    parse_parent_configurations_with_expected_configuration(input, None)
}

fn parse_parent_configurations_for_configuration(
    input: &[u8],
    configuration_uuid: &str,
) -> SupportFacts {
    parse_parent_configurations_with_expected_configuration(input, Some(configuration_uuid))
}

fn parse_parent_configurations_with_expected_configuration(
    input: &[u8],
    expected_configuration_uuid: Option<&str>,
) -> SupportFacts {
    let original_len = input.len();
    let (input, bom_len) = match input.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(input) => (input, 3),
        None => (input, 0),
    };
    if input.iter().all(u8::is_ascii_whitespace) {
        return removed();
    }
    let text = match std::str::from_utf8(input) {
        Ok(text) => text,
        Err(error) => {
            return unreadable(SupportParseError::new(
                SupportParseErrorKind::InvalidUtf8,
                bom_len + error.valid_up_to(),
                "input",
            ));
        }
    };
    let root = match AstParser::new(text, bom_len).parse_document() {
        Ok(root) => root,
        Err(error) => return unreadable(error),
    };
    match decode_certified_v6(root, original_len, expected_configuration_uuid) {
        Ok(facts) => facts,
        Err(error) => unreadable(error),
    }
}

fn decode_certified_v6(
    root: AstValue,
    input_len: usize,
    expected_configuration_uuid: Option<&str>,
) -> Result<SupportFacts, SupportParseError> {
    let AstValue::List { values, offset } = root else {
        return Err(SupportParseError::new(
            SupportParseErrorKind::UnexpectedToken,
            root.offset(),
            "root list",
        ));
    };
    let cursor = ProfileCursor::new(&values, offset, input_len);
    let version = cursor.atom(0, "format version")?;
    if version.value != "6" {
        return Err(cursor.error(
            SupportParseErrorKind::UnsupportedVersion,
            version.offset,
            "format version",
        ));
    }
    let global = match cursor.atom(1, "editing flag")?.value.as_str() {
        "0" => true,
        "1" => false,
        _ => {
            let value = cursor.atom(1, "editing flag")?;
            return Err(cursor.error(
                SupportParseErrorKind::UnknownCode,
                value.offset,
                "editing flag",
            ));
        }
    };
    let vendor_count_atom = cursor.atom(2, "vendor count")?;
    let vendor_count = vendor_count_atom.value.parse::<usize>().map_err(|_| {
        cursor.error(
            SupportParseErrorKind::UnknownCode,
            vendor_count_atom.offset,
            "vendor count",
        )
    })?;
    match vendor_count {
        0 => {
            if values.len() != 3 {
                return Err(cursor.error(
                    SupportParseErrorKind::TrailingData,
                    values[3].offset(),
                    "removed profile",
                ));
            }
            return Ok(if global {
                removed()
            } else {
                SupportFacts {
                    source: SupportSourceState::Parsed,
                    object_rules: BTreeMap::new(),
                    global_editing_enabled: Some(false),
                    configuration_rule: None,
                    vendors: Vec::new(),
                    multi_vendor_semantics_unproven: false,
                }
            });
        }
        count if count > MAX_VENDOR_COUNT => {
            return Err(cursor.error(
                SupportParseErrorKind::UnsupportedLayout,
                vendor_count_atom.offset,
                "vendor count",
            ))
        }
        _ => {}
    }
    let (next, vendors) = parse_vendor_blocks(
        &values,
        3,
        vendor_count,
        input_len,
        expected_configuration_uuid,
    )?;
    if next != values.len() {
        return Err(SupportParseError::new(
            SupportParseErrorKind::TrailingData,
            values[next].offset(),
            "vendor blocks",
        ));
    }
    let (configuration_rule, object_rules) = match vendors.as_slice() {
        [vendor] => (
            vendor.configuration_rule.clone(),
            vendor.object_rules.clone(),
        ),
        _ => (None, BTreeMap::new()),
    };
    Ok(SupportFacts {
        source: SupportSourceState::Parsed,
        object_rules,
        global_editing_enabled: Some(global),
        configuration_rule,
        multi_vendor_semantics_unproven: vendors.len() > 1,
        vendors,
    })
}

fn parse_vendor_blocks(
    values: &[AstValue],
    index: usize,
    remaining_vendors: usize,
    input_len: usize,
    expected_configuration_uuid: Option<&str>,
) -> Result<(usize, Vec<SupportVendor>), SupportParseError> {
    let header_end = index.checked_add(7).ok_or_else(|| {
        SupportParseError::new(SupportParseErrorKind::Truncated, input_len, "vendor block")
    })?;
    if header_end > values.len() {
        return Err(SupportParseError::new(
            SupportParseErrorKind::Truncated,
            input_len,
            "vendor block",
        ));
    }
    let provider_uuid = parse_uuid(&values[index], "provider UUID")?;
    let vendor_flag = match atom_value(&values[index + 1], "vendor flag")?
        .value
        .as_str()
    {
        "0" => false,
        "1" => true,
        _ => {
            return Err(SupportParseError::new(
                SupportParseErrorKind::UnknownCode,
                values[index + 1].offset(),
                "vendor flag",
            ))
        }
    };
    let vendor_configuration_uuid = parse_uuid(&values[index + 2], "vendor configuration UUID")?;
    let version = match &values[index + 3] {
        AstValue::String(value) => value.value.clone(),
        value => {
            return Err(SupportParseError::new(
                SupportParseErrorKind::UnexpectedToken,
                value.offset(),
                "vendor version",
            ))
        }
    };
    let vendor = match &values[index + 4] {
        AstValue::String(value) => value.value.clone(),
        value => {
            return Err(SupportParseError::new(
                SupportParseErrorKind::UnexpectedToken,
                value.offset(),
                "vendor name",
            ))
        }
    };
    let name = match &values[index + 5] {
        AstValue::String(value) => value.value.clone(),
        value => {
            return Err(SupportParseError::new(
                SupportParseErrorKind::UnexpectedToken,
                value.offset(),
                "vendor configuration name",
            ))
        }
    };
    let rule_count_atom = atom_value(&values[index + 6], "rule count")?;
    let rule_count = rule_count_atom.value.parse::<usize>().map_err(|_| {
        SupportParseError::new(
            SupportParseErrorKind::UnknownCode,
            rule_count_atom.offset,
            "rule count",
        )
    })?;
    if rule_count > MAX_RULES_PER_VENDOR {
        return Err(SupportParseError::new(
            SupportParseErrorKind::UnsupportedLayout,
            rule_count_atom.offset,
            "rule count",
        ));
    }

    let mut matches = Vec::new();
    let mut first_rule_error = None;
    let mut trailing_candidate = None;
    for has_configuration_rule in [false, true] {
        if has_configuration_rule && rule_count == 0 {
            continue;
        }
        let field_count = rule_count
            .checked_mul(4)
            .and_then(|count| count.checked_sub(usize::from(has_configuration_rule)))
            .ok_or_else(|| {
                SupportParseError::new(
                    SupportParseErrorKind::UnsupportedLayout,
                    rule_count_atom.offset,
                    "rule count",
                )
            })?;
        let rules_end = match header_end.checked_add(field_count) {
            Some(end) => end,
            None => continue,
        };
        if rules_end > values.len() {
            continue;
        }
        let parsed = parse_vendor_rules(
            &values[header_end..rules_end],
            rule_count,
            has_configuration_rule,
            expected_configuration_uuid,
        );
        let (configuration_rule, object_rules) = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                retain_furthest_error(&mut first_rule_error, error);
                continue;
            }
        };
        let vendor = SupportVendor {
            provider_uuid: provider_uuid.clone(),
            vendor_flag,
            vendor_configuration_uuid: vendor_configuration_uuid.clone(),
            version: version.clone(),
            vendor: vendor.clone(),
            name: name.clone(),
            configuration_rule,
            object_rules,
        };
        if remaining_vendors == 1 {
            if rules_end == values.len() {
                matches.push((rules_end, vec![vendor]));
            } else if rules_end < values.len() {
                trailing_candidate = Some(values[rules_end].offset());
            }
        } else {
            match parse_vendor_blocks(
                values,
                rules_end,
                remaining_vendors - 1,
                input_len,
                expected_configuration_uuid,
            ) {
                Ok((end, mut rest)) => {
                    let mut all = vec![vendor];
                    all.append(&mut rest);
                    matches.push((end, all));
                }
                Err(error) => retain_furthest_error(&mut first_rule_error, error),
            }
        }
    }
    match matches.len() {
        1 => Ok(matches.pop().expect("one candidate")),
        0 => {
            let trailing_error = trailing_candidate.map(|offset| {
                SupportParseError::new(SupportParseErrorKind::TrailingData, offset, "vendor block")
            });
            match (first_rule_error, trailing_error) {
                (Some(error), Some(trailing))
                    if error.offset.unwrap_or(0) >= trailing.offset.unwrap_or(0) =>
                {
                    Err(error)
                }
                (Some(_), Some(trailing)) => Err(trailing),
                (Some(error), None) => Err(error),
                (None, Some(trailing)) => Err(trailing),
                (None, None) if header_end >= values.len() => Err(SupportParseError::new(
                    SupportParseErrorKind::Truncated,
                    input_len,
                    "rule collection",
                )),
                (None, None) => Err(SupportParseError::new(
                    SupportParseErrorKind::UnsupportedLayout,
                    values[header_end].offset(),
                    "rule collection",
                )),
            }
        }
        _ => Err(SupportParseError::new(
            SupportParseErrorKind::UnsupportedLayout,
            values[header_end].offset(),
            "ambiguous vendor rule layout",
        )),
    }
}

fn retain_furthest_error(slot: &mut Option<SupportParseError>, candidate: SupportParseError) {
    let replace = slot
        .as_ref()
        .is_none_or(|current| candidate.offset.unwrap_or(0) >= current.offset.unwrap_or(0));
    if replace {
        *slot = Some(candidate);
    }
}

fn parse_vendor_rules(
    values: &[AstValue],
    rule_count: usize,
    has_configuration_rule: bool,
    expected_configuration_uuid: Option<&str>,
) -> Result<(Option<(String, SupportRule)>, BTreeMap<String, SupportRule>), SupportParseError> {
    let (configuration_rule, object_values) = if has_configuration_rule {
        let rule = parse_configuration_rule(&values[..3], expected_configuration_uuid)?;
        (Some(rule), &values[3..])
    } else {
        (None, values)
    };
    if object_values.len() != (rule_count - usize::from(has_configuration_rule)) * 4 {
        return Err(SupportParseError::new(
            SupportParseErrorKind::UnsupportedLayout,
            values.first().map(AstValue::offset).unwrap_or(0),
            "rule collection",
        ));
    }
    let mut seen = BTreeMap::new();
    if let Some((uuid, rule)) = &configuration_rule {
        seen.insert(uuid.clone(), *rule);
    }
    let mut object_rules = BTreeMap::new();
    for rule in object_values.chunks_exact(4) {
        let (object_id, state) = parse_object_rule(rule)?;
        if seen.insert(object_id.clone(), state).is_some()
            || object_rules.insert(object_id, state).is_some()
        {
            return Err(SupportParseError::new(
                SupportParseErrorKind::DuplicateRule,
                rule[2].offset(),
                "object rule UUID",
            ));
        }
    }
    Ok((configuration_rule, object_rules))
}

fn parse_configuration_rule(
    values: &[AstValue],
    expected_configuration_uuid: Option<&str>,
) -> Result<(String, SupportRule), SupportParseError> {
    let state = parse_rule_state(&values[0], "configuration rule state")?;
    let marker = atom_value(&values[1], "configuration rule marker")?;
    if marker.value != "0" {
        return Err(SupportParseError::new(
            SupportParseErrorKind::UnknownCode,
            marker.offset,
            "configuration rule marker",
        ));
    }
    let configuration = parse_uuid(&values[2], "configuration rule UUID")?;
    if expected_configuration_uuid.is_some_and(|expected| expected != configuration) {
        return Err(SupportParseError::new(
            SupportParseErrorKind::ConflictingEvidence,
            values[2].offset(),
            "configuration rule UUID",
        ));
    }
    Ok((configuration, state))
}

fn parse_object_rule(values: &[AstValue]) -> Result<(String, SupportRule), SupportParseError> {
    let marker = atom_value(&values[1], "object rule marker")?;
    if marker.value != "0" {
        return Err(SupportParseError::new(
            SupportParseErrorKind::UnknownCode,
            marker.offset,
            "object rule marker",
        ));
    }
    let state = parse_rule_state(&values[0], "object rule state")?;
    let object = parse_uuid(&values[2], "object rule UUID")?;
    let owner = parse_uuid(&values[3], "object rule owner UUID")?;
    if object != owner {
        return Err(SupportParseError::new(
            SupportParseErrorKind::ConflictingEvidence,
            values[3].offset(),
            "object rule UUIDs",
        ));
    }
    Ok((object, state))
}

fn parse_rule_state(
    value: &AstValue,
    context: &'static str,
) -> Result<SupportRule, SupportParseError> {
    match atom_value(value, context)?.value.as_str() {
        "0" => Ok(SupportRule::Locked),
        "1" => Ok(SupportRule::Editable),
        "2" => Ok(SupportRule::Removed),
        _ => Err(SupportParseError::new(
            SupportParseErrorKind::UnknownCode,
            value.offset(),
            context,
        )),
    }
}

fn parse_uuid(value: &AstValue, context: &'static str) -> Result<String, SupportParseError> {
    let atom = atom_value(value, context)?;
    uuid::Uuid::parse_str(&atom.value)
        .map(|uuid| uuid.to_string())
        .map_err(|_| {
            SupportParseError::new(SupportParseErrorKind::InvalidUuid, atom.offset, context)
        })
}

fn atom_value<'a>(
    value: &'a AstValue,
    context: &'static str,
) -> Result<&'a AstAtom, SupportParseError> {
    match value {
        AstValue::Atom(atom) => Ok(atom),
        _ => Err(SupportParseError::new(
            SupportParseErrorKind::UnexpectedToken,
            value.offset(),
            context,
        )),
    }
}

fn absent() -> SupportFacts {
    SupportFacts {
        source: SupportSourceState::Absent,
        object_rules: BTreeMap::new(),
        global_editing_enabled: None,
        configuration_rule: None,
        vendors: Vec::new(),
        multi_vendor_semantics_unproven: false,
    }
}

fn removed() -> SupportFacts {
    SupportFacts {
        source: SupportSourceState::Removed,
        object_rules: BTreeMap::new(),
        global_editing_enabled: Some(true),
        configuration_rule: None,
        vendors: Vec::new(),
        multi_vendor_semantics_unproven: false,
    }
}

fn unreadable(error: SupportParseError) -> SupportFacts {
    SupportFacts {
        source: SupportSourceState::Unreadable { error },
        object_rules: BTreeMap::new(),
        global_editing_enabled: None,
        configuration_rule: None,
        vendors: Vec::new(),
        multi_vendor_semantics_unproven: false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AstValue {
    Atom(AstAtom),
    String(AstAtom),
    List {
        values: Vec<AstValue>,
        offset: usize,
    },
}

impl AstValue {
    fn offset(&self) -> usize {
        match self {
            Self::Atom(value) | Self::String(value) => value.offset,
            Self::List { offset, .. } => *offset,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AstAtom {
    value: String,
    offset: usize,
}

struct AstParser<'a> {
    input: &'a str,
    position: usize,
    base_offset: usize,
}

impl<'a> AstParser<'a> {
    fn new(input: &'a str, base_offset: usize) -> Self {
        Self {
            input,
            position: 0,
            base_offset,
        }
    }

    fn parse_document(mut self) -> Result<AstValue, SupportParseError> {
        self.skip_whitespace();
        let value = self.parse_value(0)?;
        self.skip_whitespace();
        if self.position != self.input.len() {
            return Err(self.error(SupportParseErrorKind::TrailingData, "document"));
        }
        Ok(value)
    }

    fn parse_value(&mut self, nesting_depth: usize) -> Result<AstValue, SupportParseError> {
        self.skip_whitespace();
        match self.byte() {
            Some(b'{') => self.parse_list(nesting_depth),
            Some(b'"') => self.parse_string(),
            Some(_) => self.parse_atom(),
            None => Err(self.error(SupportParseErrorKind::Truncated, "value")),
        }
    }

    fn parse_list(&mut self, nesting_depth: usize) -> Result<AstValue, SupportParseError> {
        let nesting_depth = nesting_depth.checked_add(1).ok_or_else(|| {
            self.error(SupportParseErrorKind::NestingLimitExceeded, "list nesting")
        })?;
        if nesting_depth > MAX_NAVIGATION_NESTING_DEPTH {
            return Err(self.error(SupportParseErrorKind::NestingLimitExceeded, "list nesting"));
        }
        let offset = self.base_offset + self.position;
        self.position += 1;
        self.skip_whitespace();
        let mut values = Vec::new();
        if self.consume(b'}') {
            return Ok(AstValue::List { values, offset });
        }
        loop {
            values.push(self.parse_value(nesting_depth)?);
            self.skip_whitespace();
            if self.consume(b'}') {
                return Ok(AstValue::List { values, offset });
            }
            if !self.consume(b',') {
                let kind = if self.byte().is_none() {
                    SupportParseErrorKind::Truncated
                } else {
                    SupportParseErrorKind::UnexpectedToken
                };
                return Err(self.error(kind, "list separator"));
            }
            self.skip_whitespace();
            if self.byte() == Some(b'}') {
                return Err(self.error(SupportParseErrorKind::UnexpectedToken, "trailing comma"));
            }
        }
    }

    fn parse_string(&mut self) -> Result<AstValue, SupportParseError> {
        let offset = self.base_offset + self.position;
        self.position += 1;
        let mut value = String::new();
        let mut segment_start = self.position;
        while self.position < self.input.len() {
            match self.byte().expect("position is in bounds") {
                b'\\' => {
                    return Err(self.error(SupportParseErrorKind::InvalidEscape, "string escape"))
                }
                b'"' => {
                    value.push_str(&self.input[segment_start..self.position]);
                    self.position += 1;
                    if self.byte() == Some(b'"') {
                        value.push('"');
                        self.position += 1;
                        segment_start = self.position;
                        continue;
                    }
                    return Ok(AstValue::String(AstAtom { value, offset }));
                }
                _ => {
                    self.position += self.input[self.position..]
                        .chars()
                        .next()
                        .expect("position is in bounds")
                        .len_utf8();
                }
            }
        }
        Err(self.error(SupportParseErrorKind::UnterminatedString, "string"))
    }

    fn parse_atom(&mut self) -> Result<AstValue, SupportParseError> {
        let start = self.position;
        while matches!(self.byte(), Some(byte) if !matches!(byte, b',' | b'}' | b'{' | b'"')) {
            self.position += self.input[self.position..]
                .chars()
                .next()
                .expect("position is in bounds")
                .len_utf8();
        }
        let raw = self.input[start..self.position].trim();
        if raw.is_empty() {
            return Err(SupportParseError::new(
                SupportParseErrorKind::UnexpectedToken,
                self.base_offset + start,
                "atom",
            ));
        }
        let offset = start + self.input[start..self.position].find(raw).unwrap_or(0);
        Ok(AstValue::Atom(AstAtom {
            value: raw.to_string(),
            offset: self.base_offset + offset,
        }))
    }

    fn skip_whitespace(&mut self) {
        while self
            .input
            .get(self.position..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(char::is_whitespace)
        {
            self.position += self.input[self.position..]
                .chars()
                .next()
                .expect("checked above")
                .len_utf8();
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.byte() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn byte(&self) -> Option<u8> {
        self.input.as_bytes().get(self.position).copied()
    }

    fn error(&self, kind: SupportParseErrorKind, context: &'static str) -> SupportParseError {
        SupportParseError::new(kind, self.base_offset + self.position, context)
    }
}

struct ProfileCursor<'a> {
    values: &'a [AstValue],
    root_offset: usize,
    eof_offset: usize,
}

impl<'a> ProfileCursor<'a> {
    fn new(values: &'a [AstValue], root_offset: usize, eof_offset: usize) -> Self {
        Self {
            values,
            root_offset,
            eof_offset,
        }
    }

    fn atom(&self, index: usize, context: &'static str) -> Result<&'a AstAtom, SupportParseError> {
        let value = self.values.get(index).ok_or_else(|| {
            self.error(SupportParseErrorKind::Truncated, self.eof_offset, context)
        })?;
        atom_value(value, context)
    }

    fn string(
        &self,
        index: usize,
        context: &'static str,
    ) -> Result<&'a AstAtom, SupportParseError> {
        match self.values.get(index) {
            Some(AstValue::String(value)) => Ok(value),
            Some(value) => Err(self.error(
                SupportParseErrorKind::UnexpectedToken,
                value.offset(),
                context,
            )),
            None => Err(self.error(SupportParseErrorKind::Truncated, self.eof_offset, context)),
        }
    }

    fn uuid(&self, index: usize, context: &'static str) -> Result<String, SupportParseError> {
        parse_uuid(
            self.values.get(index).ok_or_else(|| {
                self.error(SupportParseErrorKind::Truncated, self.eof_offset, context)
            })?,
            context,
        )
    }

    fn error(
        &self,
        kind: SupportParseErrorKind,
        offset: usize,
        context: &'static str,
    ) -> SupportParseError {
        SupportParseError::new(kind, offset, context)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_parent_configurations, read_support_facts,
        read_support_facts_bytes_for_configuration, EffectiveSupportRule, SupportParseErrorKind,
        SupportSourceState,
    };
    use crate::domain::navigation::Authorability;
    use std::fs;

    const PROVIDER: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    const SECOND_PROVIDER: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
    const VENDOR_CONFIGURATION: &str = "cccccccc-cccc-cccc-cccc-cccccccccccc";
    const SECOND_VENDOR_CONFIGURATION: &str = "dddddddd-dddd-dddd-dddd-dddddddddddd";
    const CONFIGURATION: &str = "11111111-1111-1111-1111-111111111111";
    const FIRST: &str = "22222222-2222-2222-2222-222222222222";
    const SECOND: &str = "33333333-3333-3333-3333-333333333333";

    #[test]
    fn real_v6_vendor_flags_and_rule_count_are_parsed() {
        for vendor_flag in ["0", "1"] {
            let facts =
                parse_parent_configurations(payload(vendor_flag, "0", "0", "0", "2").as_bytes());
            assert!(matches!(facts.source, SupportSourceState::Parsed));
            assert_eq!(facts.vendors().len(), 1);
            assert_eq!(facts.vendors()[0].vendor_flag, vendor_flag == "1");
            assert_eq!(facts.authorability_for(FIRST), Authorability::SupportLocked);
            assert_eq!(
                facts.effective_rule_for(SECOND),
                EffectiveSupportRule::Locked
            );
        }
    }

    #[test]
    fn configuration_rule_is_checked_against_configuration_xml_uuid() {
        let input = payload("0", "0", "1", "0", "1");
        let valid =
            read_support_facts_bytes_for_configuration(Some(input.as_bytes()), CONFIGURATION);
        assert_eq!(valid.authorability_for(FIRST), Authorability::SupportLocked);

        let mismatch = read_support_facts_bytes_for_configuration(
            Some(input.as_bytes()),
            "99999999-9999-9999-9999-999999999999",
        );
        assert_eq!(
            unreadable_kind(&mismatch),
            SupportParseErrorKind::ConflictingEvidence
        );
        assert_eq!(
            unreadable_error(&mismatch).offset,
            Some(input.find(CONFIGURATION).unwrap())
        );
    }

    #[test]
    fn declared_rule_count_and_record_markers_are_consumed_exactly() {
        let truncated = payload("0", "0", "0", "0", "1").replace(",3,0,0,", ",4,0,0,");
        assert_kind(
            truncated.as_bytes(),
            SupportParseErrorKind::UnsupportedLayout,
        );

        let invalid_marker = payload("0", "0", "0", "0", "1").replace(",0,0,2222", ",0,2,2222");
        assert_kind(
            invalid_marker.as_bytes(),
            SupportParseErrorKind::UnknownCode,
        );

        let mut trailing = payload("0", "0", "0", "0", "1");
        trailing.pop();
        trailing.push_str(",surplus}");
        assert_kind(trailing.as_bytes(), SupportParseErrorKind::TrailingData);
    }

    #[test]
    fn repeated_object_uuid_and_conflicting_owner_fail_closed_at_record_offsets() {
        let duplicate = payload("0", "0", "0", "0", "1").replace(SECOND, FIRST);
        assert_kind(duplicate.as_bytes(), SupportParseErrorKind::DuplicateRule);

        let conflicting = payload("0", "0", "0", "0", "1").replacen(
            &format!(",{SECOND},{SECOND}}}"),
            &format!(",{SECOND},{CONFIGURATION}}}"),
            1,
        );
        assert_kind(
            conflicting.as_bytes(),
            SupportParseErrorKind::ConflictingEvidence,
        );
    }

    #[test]
    fn multiple_vendor_facts_are_retained_but_never_grant_authorability() {
        let input = format!(
            "{{6,0,2,{PROVIDER},0,{VENDOR_CONFIGURATION},\"1.0\",\"Vendor\",\"One\",1,0,0,{CONFIGURATION},{SECOND_PROVIDER},1,{SECOND_VENDOR_CONFIGURATION},\"2.0\",\"Vendor\",\"Two\",1,1,0,{CONFIGURATION}}}"
        );
        let facts =
            read_support_facts_bytes_for_configuration(Some(input.as_bytes()), CONFIGURATION);
        assert!(matches!(facts.source, SupportSourceState::Parsed));
        assert_eq!(facts.vendors().len(), 2);
        assert_eq!(
            facts.authorability_for(FIRST),
            Authorability::UnknownReadOnly
        );
    }

    #[test]
    fn tracked_on_support_fixture_uses_the_real_v6_layout() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/meta-compile/fixtures/on-support/Ext/ParentConfigurations.bin"
        ));
        let facts = read_support_facts_bytes_for_configuration(Some(bytes), CONFIGURATION);
        assert!(matches!(facts.source, SupportSourceState::Parsed));
        assert_eq!(facts.authorability_for(FIRST), Authorability::SupportLocked);
        assert_eq!(
            facts.effective_rule_for(SECOND),
            EffectiveSupportRule::Locked
        );
    }

    #[test]
    fn missing_snapshot_support_is_absent_and_oversize_fails_closed() {
        assert!(matches!(
            super::read_support_facts_bytes(None).source,
            SupportSourceState::Absent
        ));
        let facts = super::read_support_facts_bytes(Some(&vec![b' '; 1024 * 1024 + 1]));
        assert_eq!(
            unreadable_kind(&facts),
            SupportParseErrorKind::InputTooLarge
        );
        assert_eq!(
            facts.authorability_for(FIRST),
            Authorability::UnknownReadOnly
        );
    }

    #[test]
    fn filesystem_reader_keeps_existing_guard_path_for_regular_real_layout() {
        let path = std::env::temp_dir().join(format!("unica-support-real-{}", std::process::id()));
        fs::write(&path, payload("1", "0", "0", "0", "1")).unwrap();
        let facts = read_support_facts(&path);
        assert_eq!(facts.authorability_for(FIRST), Authorability::SupportLocked);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn second_vendor_invalid_uuid_keeps_its_specific_original_offset() {
        let invalid = "not-a-uuid";
        let input = format!(
            "{{6,1,2,{PROVIDER},0,{VENDOR_CONFIGURATION},\"1.0\",\"Vendor\",\"VendorConf\",0,{invalid},0,{VENDOR_CONFIGURATION},\"1.0\",\"Vendor\",\"VendorConf\",0}}"
        );

        let facts = parse_parent_configurations(input.as_bytes());
        let error = unreadable_error(&facts);

        assert_eq!(error.kind, SupportParseErrorKind::InvalidUuid);
        assert_eq!(error.offset, Some(input.find(invalid).unwrap()));
    }

    #[test]
    fn parent_configurations_ast_nesting_is_bounded_before_recursive_allocation() {
        let at_limit = nested_lists(crate::domain::navigation_limits::MAX_NAVIGATION_NESTING_DEPTH);
        assert!(super::AstParser::new(&at_limit, 0).parse_document().is_ok());

        let over_limit =
            nested_lists(crate::domain::navigation_limits::MAX_NAVIGATION_NESTING_DEPTH + 1);
        let facts = parse_parent_configurations(over_limit.as_bytes());
        assert_eq!(
            unreadable_kind(&facts),
            SupportParseErrorKind::NestingLimitExceeded
        );
        assert_eq!(unreadable_error(&facts).context, "list nesting");
    }

    fn payload(
        vendor_flag: &str,
        global: &str,
        configuration_state: &str,
        first_state: &str,
        second_state: &str,
    ) -> String {
        format!(
            "{{6,{global},1,{PROVIDER},{vendor_flag},{VENDOR_CONFIGURATION},\"1.0\",\"Vendor\",\"VendorConf\",3,{configuration_state},0,{CONFIGURATION},{first_state},0,{FIRST},{FIRST},{second_state},0,{SECOND},{SECOND}}}"
        )
    }

    fn nested_lists(depth: usize) -> String {
        format!("{}0{}", "{".repeat(depth), "}".repeat(depth))
    }

    fn assert_kind(input: &[u8], expected: SupportParseErrorKind) {
        let facts = parse_parent_configurations(input);
        assert_eq!(unreadable_kind(&facts), expected);
    }

    fn unreadable_kind(facts: &super::SupportFacts) -> SupportParseErrorKind {
        unreadable_error(facts).kind
    }

    fn unreadable_error(facts: &super::SupportFacts) -> &super::SupportParseError {
        match &facts.source {
            SupportSourceState::Unreadable { error } => error,
            source => panic!("expected unreadable state, got {source:?}"),
        }
    }
}
