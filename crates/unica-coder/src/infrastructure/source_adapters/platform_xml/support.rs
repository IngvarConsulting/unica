use crate::domain::navigation::Authorability;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

const MAX_PARENT_CONFIGURATIONS_BYTES: u64 = 1024 * 1024;
const CERTIFIED_VENDOR_CAPABILITY: &str = "0";
const CERTIFIED_RULE_SCHEMA: &str = "3";
const CERTIFIED_RULE_SCOPE: &str = "1";
const CERTIFIED_OBJECT_RULE_FIELDS: usize = 4;
const CERTIFIED_OBJECT_RULE_COUNT: usize = 2;
const CERTIFIED_CONFIGURATION_RULE_FIELDS: usize = 3;

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
    Unreadable,
}

impl EffectiveSupportRule {
    pub(crate) fn authorability(self) -> Authorability {
        match self {
            Self::Absent | Self::Removed | Self::Editable => Authorability::Authorable,
            Self::Locked => Authorability::SupportLocked,
            Self::ConfigurationReadOnly => Authorability::ConfigurationReadOnly,
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
    pub(crate) version: String,
    pub(crate) vendor: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportFacts {
    pub(crate) source: SupportSourceState,
    pub(crate) object_rules: BTreeMap<String, SupportRule>,
    global_editing_enabled: Option<bool>,
    configuration_rule: Option<(String, SupportRule)>,
    vendors: Vec<SupportVendor>,
}

impl SupportFacts {
    pub(crate) fn effective_rule_for(&self, object: &str) -> EffectiveSupportRule {
        match self.source {
            SupportSourceState::Absent => return EffectiveSupportRule::Absent,
            SupportSourceState::Removed => return EffectiveSupportRule::Removed,
            SupportSourceState::Unreadable { .. } => return EffectiveSupportRule::Unreadable,
            SupportSourceState::Parsed => {}
        }
        if self.global_editing_enabled == Some(false) {
            return EffectiveSupportRule::ConfigurationReadOnly;
        }
        let object = object.to_ascii_lowercase();
        let configuration = self.configuration_rule.as_ref().map(|(_, rule)| *rule);
        let object_rule = self
            .object_rules
            .get(&object)
            .copied()
            .or_else(|| {
                self.configuration_rule.as_ref().and_then(|(uuid, rule)| {
                    (uuid == &object).then_some(*rule)
                })
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
        Err(_) => return unreadable(SupportParseError::unknown(SupportParseErrorKind::Io, "metadata")),
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
        Err(_) => return unreadable(SupportParseError::unknown(SupportParseErrorKind::Io, "open")),
    };
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    if file
        .by_ref()
        .take(MAX_PARENT_CONFIGURATIONS_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return unreadable(SupportParseError::unknown(SupportParseErrorKind::Io, "read"));
    }
    if bytes.len() as u64 > MAX_PARENT_CONFIGURATIONS_BYTES {
        return unreadable(SupportParseError::unknown(
            SupportParseErrorKind::InputTooLarge,
            "support file",
        ));
    }
    parse_parent_configurations(&bytes)
}

pub(crate) fn parse_parent_configurations(input: &[u8]) -> SupportFacts {
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
    match decode_certified_v6(root, original_len) {
        Ok(facts) => facts,
        Err(error) => unreadable(error),
    }
}

fn decode_certified_v6(root: AstValue, input_len: usize) -> Result<SupportFacts, SupportParseError> {
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
        return Err(cursor.error(SupportParseErrorKind::UnsupportedVersion, version.offset, "format version"));
    }
    let global = match cursor.atom(1, "editing flag")?.value.as_str() {
        "0" => true,
        "1" => false,
        _ => {
            let value = cursor.atom(1, "editing flag")?;
            return Err(cursor.error(SupportParseErrorKind::UnknownCode, value.offset, "editing flag"));
        }
    };
    let vendor_count = cursor.atom(2, "vendor count")?;
    match vendor_count.value.as_str() {
        "0" => {
            if values.len() != 3 {
                return Err(cursor.error(SupportParseErrorKind::TrailingData, values[3].offset(), "removed profile"));
            }
            return Ok(SupportFacts {
                source: SupportSourceState::Parsed,
                object_rules: BTreeMap::new(),
                global_editing_enabled: Some(global),
                configuration_rule: None,
                vendors: Vec::new(),
            });
        }
        "1" => {}
        _ => return Err(cursor.error(SupportParseErrorKind::UnsupportedLayout, vendor_count.offset, "vendor count")),
    }

    let provider_id = cursor.uuid(3, "provider UUID")?;
    let capability = cursor.atom(4, "provider capability")?;
    if capability.value != CERTIFIED_VENDOR_CAPABILITY {
        return Err(cursor.error(SupportParseErrorKind::UnknownCode, capability.offset, "provider capability"));
    }
    cursor.uuid(5, "provider configuration UUID")?;
    let version = cursor.string(6, "vendor version")?.value.clone();
    let vendor = cursor.string(7, "vendor name")?.value.clone();
    let name = cursor.string(8, "vendor configuration name")?.value.clone();
    let schema = cursor.atom(9, "rule schema")?;
    if schema.value != CERTIFIED_RULE_SCHEMA {
        return Err(cursor.error(SupportParseErrorKind::UnknownCode, schema.offset, "rule schema"));
    }
    let scope = cursor.atom(10, "rule scope")?;
    if scope.value != CERTIFIED_RULE_SCOPE {
        return Err(cursor.error(SupportParseErrorKind::UnknownCode, scope.offset, "rule scope"));
    }
    let tail = values.get(11..).unwrap_or_default();
    let layout = CertifiedTailLayout::for_values(tail);
    let expected_fields = layout.field_count();
    if tail.len() < expected_fields {
        return Err(cursor.error(
            SupportParseErrorKind::Truncated,
            input_len,
            "rule collection",
        ));
    }
    if tail.len() > expected_fields {
        return Err(cursor.error(
            SupportParseErrorKind::TrailingData,
            tail[expected_fields].offset(),
            "certified rule collection",
        ));
    }
    let (configuration_rule, rule_values) = match layout {
        CertifiedTailLayout::ObjectRules => (None, tail),
        CertifiedTailLayout::ConfigurationAndObjectRules => (
            Some(parse_configuration_rule(&tail[..CERTIFIED_CONFIGURATION_RULE_FIELDS])?),
            &tail[CERTIFIED_CONFIGURATION_RULE_FIELDS..],
        ),
    };
    let mut object_rules = BTreeMap::new();
    for rule in rule_values.chunks_exact(4) {
        let (object_id, state) = parse_object_rule(rule)?;
        if object_rules.insert(object_id, state).is_some() {
            return Err(SupportParseError::new(
                SupportParseErrorKind::DuplicateRule,
                rule[2].offset(),
                "object rule UUID",
            ));
        }
    }
    debug_assert_eq!(
        rule_values.len(),
        CERTIFIED_OBJECT_RULE_FIELDS * CERTIFIED_OBJECT_RULE_COUNT
    );
    let _ = provider_id;
    Ok(SupportFacts {
        source: SupportSourceState::Parsed,
        object_rules,
        global_editing_enabled: Some(global),
        configuration_rule,
        vendors: vec![SupportVendor { version, vendor, name }],
    })
}

fn parse_configuration_rule(values: &[AstValue]) -> Result<(String, SupportRule), SupportParseError> {
    let state = parse_rule_state(&values[0], "configuration rule state")?;
    let object = parse_uuid(&values[1], "configuration rule UUID")?;
    let owner = parse_uuid(&values[2], "configuration rule owner UUID")?;
    if object != owner {
        return Err(SupportParseError::new(
            SupportParseErrorKind::ConflictingEvidence,
            values[2].offset(),
            "configuration rule UUIDs",
        ));
    }
    Ok((object, state))
}

fn parse_object_rule(values: &[AstValue]) -> Result<(String, SupportRule), SupportParseError> {
    let kind = atom_value(&values[0], "object rule kind")?;
    if !matches!(kind.value.as_str(), "0" | "2") {
        return Err(SupportParseError::new(
            SupportParseErrorKind::UnknownCode,
            kind.offset,
            "object rule kind",
        ));
    }
    let state = parse_rule_state(&values[1], "object rule state")?;
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

fn parse_rule_state(value: &AstValue, context: &'static str) -> Result<SupportRule, SupportParseError> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CertifiedTailLayout {
    ObjectRules,
    ConfigurationAndObjectRules,
}

impl CertifiedTailLayout {
    fn for_values(values: &[AstValue]) -> Self {
        if values.get(1).is_some_and(is_uuid_atom) && values.get(2).is_some_and(is_uuid_atom) {
            Self::ConfigurationAndObjectRules
        } else {
            Self::ObjectRules
        }
    }

    fn field_count(self) -> usize {
        let object_rules = CERTIFIED_OBJECT_RULE_FIELDS * CERTIFIED_OBJECT_RULE_COUNT;
        match self {
            Self::ObjectRules => object_rules,
            Self::ConfigurationAndObjectRules => {
                CERTIFIED_CONFIGURATION_RULE_FIELDS + object_rules
            }
        }
    }
}

fn is_uuid_atom(value: &AstValue) -> bool {
    match value {
        AstValue::Atom(atom) => uuid::Uuid::parse_str(&atom.value).is_ok(),
        AstValue::String(_) | AstValue::List { .. } => false,
    }
}

fn parse_uuid(value: &AstValue, context: &'static str) -> Result<String, SupportParseError> {
    let atom = atom_value(value, context)?;
    uuid::Uuid::parse_str(&atom.value)
        .map(|uuid| uuid.to_string())
        .map_err(|_| SupportParseError::new(SupportParseErrorKind::InvalidUuid, atom.offset, context))
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
    }
}

fn removed() -> SupportFacts {
    SupportFacts {
        source: SupportSourceState::Removed,
        object_rules: BTreeMap::new(),
        global_editing_enabled: Some(true),
        configuration_rule: None,
        vendors: Vec::new(),
    }
}

fn unreadable(error: SupportParseError) -> SupportFacts {
    SupportFacts {
        source: SupportSourceState::Unreadable { error },
        object_rules: BTreeMap::new(),
        global_editing_enabled: None,
        configuration_rule: None,
        vendors: Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AstValue {
    Atom(AstAtom),
    String(AstAtom),
    List { values: Vec<AstValue>, offset: usize },
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
        Self { input, position: 0, base_offset }
    }

    fn parse_document(mut self) -> Result<AstValue, SupportParseError> {
        self.skip_whitespace();
        let value = self.parse_value()?;
        self.skip_whitespace();
        if self.position != self.input.len() {
            return Err(self.error(SupportParseErrorKind::TrailingData, "document"));
        }
        Ok(value)
    }

    fn parse_value(&mut self) -> Result<AstValue, SupportParseError> {
        self.skip_whitespace();
        match self.byte() {
            Some(b'{') => self.parse_list(),
            Some(b'"') => self.parse_string(),
            Some(_) => self.parse_atom(),
            None => Err(self.error(SupportParseErrorKind::Truncated, "value")),
        }
    }

    fn parse_list(&mut self) -> Result<AstValue, SupportParseError> {
        let offset = self.base_offset + self.position;
        self.position += 1;
        self.skip_whitespace();
        let mut values = Vec::new();
        if self.consume(b'}') {
            return Ok(AstValue::List { values, offset });
        }
        loop {
            values.push(self.parse_value()?);
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
                b'\\' => return Err(self.error(SupportParseErrorKind::InvalidEscape, "string escape")),
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
        Self { values, root_offset, eof_offset }
    }

    fn atom(&self, index: usize, context: &'static str) -> Result<&'a AstAtom, SupportParseError> {
        let value = self.values.get(index).ok_or_else(|| self.error(
            SupportParseErrorKind::Truncated,
            self.eof_offset,
            context,
        ))?;
        atom_value(value, context)
    }

    fn string(&self, index: usize, context: &'static str) -> Result<&'a AstAtom, SupportParseError> {
        match self.values.get(index) {
            Some(AstValue::String(value)) => Ok(value),
            Some(value) => Err(self.error(SupportParseErrorKind::UnexpectedToken, value.offset(), context)),
            None => Err(self.error(
                SupportParseErrorKind::Truncated,
                self.eof_offset,
                context,
            )),
        }
    }

    fn uuid(&self, index: usize, context: &'static str) -> Result<String, SupportParseError> {
        parse_uuid(self.values.get(index).ok_or_else(|| self.error(
            SupportParseErrorKind::Truncated,
            self.eof_offset,
            context,
        ))?, context)
    }

    fn error(&self, kind: SupportParseErrorKind, offset: usize, context: &'static str) -> SupportParseError {
        SupportParseError::new(kind, offset, context)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_parent_configurations, read_support_facts, EffectiveSupportRule,
        SupportParseErrorKind, SupportSourceState,
    };
    use crate::domain::navigation::Authorability;
    use std::fs;

    const PROVIDER: &str = "dddddddd-dddd-dddd-dddd-dddddddddddd";
    const VENDOR_CONFIGURATION: &str = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
    const CONFIGURATION: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    const FIRST: &str = "cccccccc-cccc-cccc-cccc-cccccccccccc";
    const SECOND: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";

    #[test]
    fn valid_header_with_garbage_body_is_unreadable() {
        let facts = parse_parent_configurations(b"{6,0,1,garbage}");
        assert!(matches!(facts.source, SupportSourceState::Unreadable { .. }));
    }

    #[test]
    fn truncated_object_rule_count_is_unreadable() {
        assert_kind(b"{6,0,2,{1,0}}", SupportParseErrorKind::UnsupportedLayout);
    }

    #[test]
    fn unsupported_version_is_typed() {
        assert_kind(compact("7", "0", "0", "0", "0", "0").as_bytes(), SupportParseErrorKind::UnsupportedVersion);
    }

    #[test]
    fn unknown_provider_capability_is_typed() {
        assert_kind(compact("6", "0", "7", "0", "0", "0").as_bytes(), SupportParseErrorKind::UnknownCode);
    }

    #[test]
    fn unknown_object_rule_blocks_authorability() {
        let facts = parse_parent_configurations(compact("6", "0", "0", "9", "0", "0").as_bytes());
        assert_eq!(unreadable_kind(&facts), SupportParseErrorKind::UnknownCode);
        assert_eq!(facts.authorability_for(FIRST), Authorability::UnknownSupportState);
    }

    #[test]
    fn valid_shaped_trailing_quadruple_is_typed_trailing_data() {
        let mut input = compact("6", "0", "0", "1", "0", "0");
        input.pop();
        input.push_str(",0,1,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa}");
        assert_kind(input.as_bytes(), SupportParseErrorKind::TrailingData);
    }

    #[test]
    fn multiple_vendors_are_rejected_by_certified_profile() {
        assert_kind(b"{6,0,2}", SupportParseErrorKind::UnsupportedLayout);
    }

    #[test]
    fn global_locked_and_object_editable_is_locked() {
        let facts = parse_parent_configurations(full("0", "1", "1").as_bytes());
        assert_eq!(facts.authorability_for(FIRST), Authorability::SupportLocked);
    }

    #[test]
    fn global_unknown_and_object_absent_is_unreadable() {
        let facts = parse_parent_configurations(full("9", "1", "1").as_bytes());
        assert_eq!(unreadable_kind(&facts), SupportParseErrorKind::UnknownCode);
        assert_eq!(facts.authorability_for(SECOND), Authorability::UnknownSupportState);
    }

    #[test]
    fn global_editable_and_object_locked_is_locked() {
        let facts = parse_parent_configurations(full("1", "0", "1").as_bytes());
        assert_eq!(facts.authorability_for(FIRST), Authorability::SupportLocked);
    }

    #[test]
    fn both_editable_are_authorable() {
        let facts = parse_parent_configurations(full("1", "1", "1").as_bytes());
        assert_eq!(facts.authorability_for(FIRST), Authorability::Authorable);
    }

    #[test]
    fn zero_vendor_payload_preserves_global_editing_semantics() {
        let editable = parse_parent_configurations(b"{6,0,0}");
        assert!(matches!(editable.source, SupportSourceState::Parsed));
        assert_eq!(
            editable.effective_rule_for(FIRST),
            EffectiveSupportRule::Absent
        );
        assert_eq!(editable.authorability_for(FIRST), Authorability::Authorable);

        let read_only = parse_parent_configurations(b"{6,1,0}");
        assert!(matches!(read_only.source, SupportSourceState::Parsed));
        assert_eq!(
            read_only.effective_rule_for(FIRST),
            EffectiveSupportRule::ConfigurationReadOnly
        );
        assert_eq!(
            read_only.authorability_for(FIRST),
            Authorability::ConfigurationReadOnly
        );
    }

    #[test]
    fn parse_error_offsets_are_original_input_spans() {
        let bom = parse_parent_configurations(b"\xEF\xBB\xBF{7,0,0}");
        assert_eq!(unreadable_error(&bom).offset, Some(4));

        let truncated_input = b"{6,0,1";
        let truncated = parse_parent_configurations(truncated_input);
        assert_eq!(unreadable_kind(&truncated), SupportParseErrorKind::Truncated);
        assert_eq!(
            unreadable_error(&truncated).offset,
            Some(truncated_input.len())
        );

        let mut extra = compact("6", "0", "0", "0", "0", "0");
        extra.pop();
        let first_extra = extra.len() + 1;
        extra.push_str(",0,1,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa}");
        let trailing = parse_parent_configurations(extra.as_bytes());
        assert_eq!(unreadable_kind(&trailing), SupportParseErrorKind::TrailingData);
        assert_eq!(unreadable_error(&trailing).offset, Some(first_extra));

        let mut configuration_extra = full("1", "1", "1");
        configuration_extra.pop();
        let configuration_first_extra = configuration_extra.len() + 1;
        configuration_extra.push_str(",surplus}");
        let configuration_trailing = parse_parent_configurations(configuration_extra.as_bytes());
        assert_eq!(
            unreadable_kind(&configuration_trailing),
            SupportParseErrorKind::TrailingData
        );
        assert_eq!(
            unreadable_error(&configuration_trailing).offset,
            Some(configuration_first_extra)
        );

        let empty_atom = parse_parent_configurations(b"\xEF\xBB\xBF{6,,0}");
        assert_eq!(unreadable_kind(&empty_atom), SupportParseErrorKind::UnexpectedToken);
        assert_eq!(unreadable_error(&empty_atom).offset, Some(6));
    }

    #[test]
    fn duplicate_and_conflicting_uuid_evidence_are_typed() {
        assert_kind(compact("6", "0", "0", "0", "0", "0").replace(SECOND, FIRST).as_bytes(), SupportParseErrorKind::DuplicateRule);
        let conflicting = compact("6", "0", "0", "0", "0", "0").replace(",bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb}", ",aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa}");
        assert_kind(conflicting.as_bytes(), SupportParseErrorKind::ConflictingEvidence);
    }

    #[test]
    fn nested_lists_are_rejected() {
        assert_kind(b"{6,0,1,{x}}", SupportParseErrorKind::UnexpectedToken);
    }

    #[test]
    fn certified_quoted_strings_preserve_utf8_commas_braces_and_doubled_quotes() {
        let input = format!(
            "{{6,0,1,{PROVIDER},0,{VENDOR_CONFIGURATION},\"1,0\",\"Поставщик {{A}} \"\"quoted\"\"\",\"Конфигурация\",3,1,0,0,{FIRST},{FIRST},0,1,{SECOND},{SECOND}}}"
        );
        let facts = parse_parent_configurations(input.as_bytes());
        assert!(matches!(facts.source, SupportSourceState::Parsed));
        assert_eq!(facts.vendors()[0].vendor, "Поставщик {A} \"quoted\"");
        assert_eq!(facts.vendors()[0].name, "Конфигурация");
    }

    #[test]
    fn bounded_reader_rejects_oversize_input() {
        let path = std::env::temp_dir().join(format!(
            "unica-support-bounded-{}",
            std::process::id()
        ));
        fs::write(&path, vec![b'x'; 1024 * 1024 + 1]).unwrap();
        let facts = read_support_facts(&path);
        assert_eq!(unreadable_kind(&facts), SupportParseErrorKind::InputTooLarge);
        fs::remove_file(path).unwrap();
    }

    fn compact(version: &str, editing: &str, capability: &str, first_kind: &str, first_state: &str, second_state: &str) -> String {
        format!(
            "{{{version},{editing},1,{PROVIDER},{capability},{VENDOR_CONFIGURATION},\"1.0\",\"Vendor\",\"VendorConf\",3,1,{first_kind},{first_state},{FIRST},{FIRST},0,{second_state},{SECOND},{SECOND}}}"
        )
    }

    fn full(configuration_state: &str, first_state: &str, second_state: &str) -> String {
        format!(
            "{{6,0,1,{PROVIDER},0,{VENDOR_CONFIGURATION},\"1.0\",\"Vendor\",\"VendorConf\",3,1,{configuration_state},{CONFIGURATION},{CONFIGURATION},0,{first_state},{FIRST},{FIRST},2,{second_state},{SECOND},{SECOND}}}"
        )
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
