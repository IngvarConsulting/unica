use crate::domain::navigation::Authorability;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

const MAX_PARENT_CONFIGURATIONS_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SupportSourceState {
    Absent,
    Removed,
    Parsed,
    Unreadable { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupportRule {
    Locked,
    Editable,
    Removed,
    Unknown,
}

impl SupportRule {
    pub(crate) fn flag(self) -> Option<u8> {
        match self {
            Self::Locked => Some(0),
            Self::Editable => Some(1),
            Self::Removed => Some(2),
            Self::Unknown => None,
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
    vendors: Vec<SupportVendor>,
}

impl SupportFacts {
    pub(crate) fn authorability_for(&self, object: &str) -> Authorability {
        match self.source {
            SupportSourceState::Absent | SupportSourceState::Removed => Authorability::Authorable,
            SupportSourceState::Unreadable { .. } => Authorability::UnknownSupportState,
            SupportSourceState::Parsed if self.global_editing_enabled == Some(false) => {
                Authorability::ConfigurationReadOnly
            }
            SupportSourceState::Parsed => match self
                .object_rules
                .get(object)
                .or_else(|| self.object_rules.get(&object.to_ascii_lowercase()))
            {
                Some(SupportRule::Locked) => Authorability::SupportLocked,
                Some(SupportRule::Editable | SupportRule::Removed) | None => {
                    Authorability::Authorable
                }
                Some(SupportRule::Unknown) => Authorability::UnknownSupportState,
            },
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
        for rule in self.object_rules.values() {
            if let Some(flag) = rule.flag() {
                counts[flag as usize] += 1;
            }
        }
        counts
    }
}

pub(crate) fn read_support_facts(bin_path: &Path) -> SupportFacts {
    let metadata = match fs::symlink_metadata(bin_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return absent(),
        Err(error) => return unreadable(format!("cannot inspect support state: {error}")),
    };
    if !metadata.file_type().is_file() {
        return unreadable("support state is not a regular file");
    }
    if metadata.len() > MAX_PARENT_CONFIGURATIONS_BYTES {
        return unreadable("support state exceeds the bounded reader limit");
    }
    let mut file = match fs::File::open(bin_path) {
        Ok(file) => file,
        Err(error) => return unreadable(format!("cannot read support state: {error}")),
    };
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    if let Err(error) = file
        .by_ref()
        .take(MAX_PARENT_CONFIGURATIONS_BYTES + 1)
        .read_to_end(&mut bytes)
    {
        return unreadable(format!("cannot read support state: {error}"));
    }
    if bytes.len() as u64 > MAX_PARENT_CONFIGURATIONS_BYTES {
        return unreadable("support state exceeds the bounded reader limit");
    }
    parse_parent_configurations(&bytes)
}

pub(crate) fn parse_parent_configurations(input: &[u8]) -> SupportFacts {
    let input = input.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(input);
    if input.iter().all(u8::is_ascii_whitespace) {
        return removed();
    }
    let text = match std::str::from_utf8(input) {
        Ok(text) => text,
        Err(_) => return unreadable("support state is not valid UTF-8"),
    };
    let root = match ValueParser::new(text).parse_document() {
        Ok(root) => root,
        Err(reason) => return unreadable(reason),
    };
    match decode_v6(root) {
        Ok(facts) => facts,
        Err(reason) => unreadable(reason),
    }
}

fn decode_v6(root: Vec<Value>) -> Result<SupportFacts, String> {
    let mut cursor = Cursor::new(root);
    if cursor.atom("format version")? != "6" {
        return Err("unsupported support-state version".to_string());
    }
    let global_editing_enabled = match cursor.atom("global editing flag")?.as_str() {
        "0" => true,
        "1" => false,
        _ => return Err("support-state global editing flag is invalid".to_string()),
    };
    let vendor_count = cursor.usize("vendor count")?;
    if vendor_count == 0 {
        cursor.finish()?;
        return Ok(removed());
    }

    let mut object_rules = BTreeMap::new();
    let mut vendors = Vec::with_capacity(vendor_count);
    for vendor_index in 0..vendor_count {
        let _provider_id = cursor.uuid("provider id")?;
        cursor.usize("provider capability")?;
        let _configuration_id = cursor.uuid("provider configuration id")?;
        let version = cursor.string("vendor version")?;
        let vendor = cursor.string("vendor name")?;
        let name = cursor.string("vendor configuration name")?;
        if cursor.usize("object rule schema marker")? != 3 {
            return Err("unsupported object-rule schema marker".to_string());
        }
        if cursor.usize("object rule scope marker")? != 1 {
            return Err("unsupported object-rule scope marker".to_string());
        }
        if cursor.starts_configuration_rule() {
            let rule = match cursor.usize("configuration rule state")? {
                0 => SupportRule::Locked,
                1 => SupportRule::Editable,
                2 => SupportRule::Removed,
                _ => SupportRule::Unknown,
            };
            let object_id = cursor.uuid("configuration rule UUID")?;
            let owner_id = cursor.uuid("configuration rule owner UUID")?;
            if owner_id != object_id {
                return Err("conflicting configuration-rule UUID evidence".to_string());
            }
            if object_rules.insert(object_id, rule).is_some() {
                return Err("duplicate configuration support rule".to_string());
            }
        }
        while !cursor.is_finished()
            && !(vendor_index + 1 < vendor_count && cursor.starts_vendor_header())
        {
            cursor.usize("object rule kind")?;
            let rule = match cursor.usize("object rule state")? {
                0 => SupportRule::Locked,
                1 => SupportRule::Editable,
                2 => SupportRule::Removed,
                _ => SupportRule::Unknown,
            };
            let object_id = cursor.uuid("object rule UUID")?;
            let owner_id = cursor.uuid("object rule owner UUID")?;
            if owner_id != object_id {
                return Err("conflicting object-rule UUID evidence".to_string());
            }
            if object_rules.insert(object_id.clone(), rule).is_some() {
                return Err("duplicate object support rule".to_string());
            }
        }
        vendors.push(SupportVendor {
            version,
            vendor,
            name,
        });
    }
    cursor.finish()?;
    Ok(SupportFacts {
        source: SupportSourceState::Parsed,
        object_rules,
        global_editing_enabled: Some(global_editing_enabled),
        vendors,
    })
}

fn absent() -> SupportFacts {
    SupportFacts {
        source: SupportSourceState::Absent,
        object_rules: BTreeMap::new(),
        global_editing_enabled: None,
        vendors: Vec::new(),
    }
}

fn removed() -> SupportFacts {
    SupportFacts {
        source: SupportSourceState::Removed,
        object_rules: BTreeMap::new(),
        global_editing_enabled: Some(true),
        vendors: Vec::new(),
    }
}

fn unreadable(reason: impl Into<String>) -> SupportFacts {
    SupportFacts {
        source: SupportSourceState::Unreadable {
            reason: reason.into(),
        },
        object_rules: BTreeMap::new(),
        global_editing_enabled: None,
        vendors: Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Atom(String),
    String(String),
    List(Vec<Value>),
}

struct ValueParser<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> ValueParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            position: 0,
        }
    }

    fn parse_document(mut self) -> Result<Vec<Value>, String> {
        self.skip_whitespace();
        let Value::List(root) = self.parse_value()? else {
            return Err("support state root must be a list".to_string());
        };
        self.skip_whitespace();
        if self.position != self.input.len() {
            return Err("support state contains trailing data".to_string());
        }
        Ok(root)
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.parse_list(),
            Some(b'"') => self.parse_string(),
            Some(_) => self.parse_atom(),
            None => Err("support state ends before a value".to_string()),
        }
    }

    fn parse_list(&mut self) -> Result<Value, String> {
        self.expect(b'{', "support-state list must start with `{`")?;
        self.skip_whitespace();
        let mut values = Vec::new();
        if self.consume(b'}') {
            return Ok(Value::List(values));
        }
        loop {
            values.push(self.parse_value()?);
            self.skip_whitespace();
            if self.consume(b'}') {
                return Ok(Value::List(values));
            }
            self.expect(b',', "support-state list values must be comma-separated")?;
            self.skip_whitespace();
            if self.peek() == Some(b'}') {
                return Err("support-state list has a trailing comma".to_string());
            }
        }
    }

    fn parse_string(&mut self) -> Result<Value, String> {
        self.expect(b'"', "support-state string must start with `\"`")?;
        let mut value = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err("support-state string is not terminated".to_string());
            };
            self.position += 1;
            if byte == b'"' {
                if self.peek() == Some(b'"') {
                    self.position += 1;
                    value.push('"');
                    continue;
                }
                return Ok(Value::String(value));
            }
            value.push(byte as char);
        }
    }

    fn parse_atom(&mut self) -> Result<Value, String> {
        let start = self.position;
        while matches!(self.peek(), Some(byte) if !matches!(byte, b',' | b'}' | b'{' | b'"')) {
            self.position += 1;
        }
        let atom = std::str::from_utf8(&self.input[start..self.position])
            .expect("parser input was valid UTF-8")
            .trim();
        if atom.is_empty() {
            return Err("support-state atom is empty".to_string());
        }
        Ok(Value::Atom(atom.to_string()))
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn expect(&mut self, expected: u8, reason: &str) -> Result<(), String> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(reason.to_string())
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }
}

struct Cursor {
    values: Vec<Value>,
    position: usize,
}

impl Cursor {
    fn new(values: Vec<Value>) -> Self {
        Self { values, position: 0 }
    }

    fn atom(&mut self, label: &str) -> Result<String, String> {
        match self.next(label)? {
            Value::Atom(value) => Ok(value),
            _ => Err(format!("{label} must be an unquoted atom")),
        }
    }

    fn string(&mut self, label: &str) -> Result<String, String> {
        match self.next(label)? {
            Value::String(value) => Ok(value),
            _ => Err(format!("{label} must be a quoted string")),
        }
    }

    fn usize(&mut self, label: &str) -> Result<usize, String> {
        self.atom(label)?
            .parse()
            .map_err(|_| format!("{label} must be an unsigned integer"))
    }

    fn uuid(&mut self, label: &str) -> Result<String, String> {
        let raw = self.atom(label)?;
        let parsed = uuid::Uuid::parse_str(&raw).map_err(|_| format!("{label} must be a UUID"))?;
        Ok(parsed.to_string())
    }

    fn finish(self) -> Result<(), String> {
        if self.position == self.values.len() {
            Ok(())
        } else {
            Err("support state contains trailing fields".to_string())
        }
    }

    fn is_finished(&self) -> bool {
        self.position == self.values.len()
    }

    fn starts_vendor_header(&self) -> bool {
        let Some(Value::Atom(provider_id)) = self.values.get(self.position) else {
            return false;
        };
        if uuid::Uuid::parse_str(provider_id).is_err() {
            return false;
        }
        let Some(Value::Atom(capability)) = self.values.get(self.position + 1) else {
            return false;
        };
        if capability.parse::<usize>().is_err() {
            return false;
        }
        let Some(Value::Atom(configuration_id)) = self.values.get(self.position + 2) else {
            return false;
        };
        if uuid::Uuid::parse_str(configuration_id).is_err() {
            return false;
        }
        matches!(
            (
                self.values.get(self.position + 3),
                self.values.get(self.position + 4),
                self.values.get(self.position + 5),
                self.values.get(self.position + 6),
            ),
            (
                Some(Value::String(_)),
                Some(Value::String(_)),
                Some(Value::String(_)),
                Some(Value::Atom(marker)),
            ) if marker == "3"
        )
    }

    fn starts_configuration_rule(&self) -> bool {
        let Some(Value::Atom(state)) = self.values.get(self.position) else {
            return false;
        };
        if state.parse::<usize>().is_err() {
            return false;
        }
        let Some(Value::Atom(object_id)) = self.values.get(self.position + 1) else {
            return false;
        };
        let Some(Value::Atom(owner_id)) = self.values.get(self.position + 2) else {
            return false;
        };
        uuid::Uuid::parse_str(object_id).is_ok()
            && uuid::Uuid::parse_str(owner_id).is_ok()
            && object_id.eq_ignore_ascii_case(owner_id)
    }

    fn next(&mut self, label: &str) -> Result<Value, String> {
        let value = self
            .values
            .get(self.position)
            .cloned()
            .ok_or_else(|| format!("support state ends before {label}"))?;
        self.position += 1;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_parent_configurations, SupportFacts, SupportRule, SupportSourceState};
    use crate::domain::navigation::Authorability;
    use std::collections::BTreeMap;

    #[test]
    fn valid_header_with_garbage_body_is_unreadable() {
        let facts = parse_parent_configurations(b"{6,0,1,garbage}");

        assert!(matches!(
            facts.source,
            SupportSourceState::Unreadable { .. }
        ));
    }

    #[test]
    fn truncated_object_rule_count_is_unreadable() {
        let facts = parse_parent_configurations(b"{6,0,2,{1,0}}");

        assert!(matches!(
            facts.source,
            SupportSourceState::Unreadable { .. }
        ));
    }

    #[test]
    fn unknown_object_rule_blocks_authorability() {
        let facts = parsed_fixture_with_unknown_rule("Document.Order");

        assert_eq!(
            facts.authorability_for("Document.Order"),
            Authorability::UnknownSupportState
        );
    }

    #[test]
    fn compact_v6_rule_section_is_fully_parsed() {
        let facts = parse_parent_configurations(
            b"{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,0,cccccccc-cccc-cccc-cccc-cccccccccccc,cccccccc-cccc-cccc-cccc-cccccccccccc,0,0,dddddddd-dddd-dddd-dddd-dddddddddddd,dddddddd-dddd-dddd-dddd-dddddddddddd}",
        );

        assert!(matches!(facts.source, SupportSourceState::Parsed));
        assert_eq!(
            facts.authorability_for("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
            Authorability::Authorable
        );
    }

    fn parsed_fixture_with_unknown_rule(object: &str) -> SupportFacts {
        SupportFacts {
            source: SupportSourceState::Parsed,
            object_rules: BTreeMap::from([(object.to_string(), SupportRule::Unknown)]),
            global_editing_enabled: Some(true),
            vendors: Vec::new(),
        }
    }
}
