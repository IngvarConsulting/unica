//! Exact supplier-block observations for checked restoration. The compatibility
//! reader and ordinary support-rule writer deliberately keep their contracts.
use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplyStagingError,
};
use unica_source_contract::SourceSetKind;

use crate::support::is_uuid_text;

#[derive(Debug)]
pub struct RestoreObjectRecord {
    pub object_uuid: String,
    /// The second UUID is retained separately: equality of the pair is not an
    /// assumption a restoration proof may make.
    pub paired_uuid: String,
    pub rule: u8,
    pub recommendation: i8,
    /// Byte range in the original marker, including its optional UTF-8 BOM.
    pub rule_span: Range<usize>,
}

#[derive(Debug)]
pub struct RestoreVendorRecord {
    pub version: String,
    pub vendor: String,
    pub name: String,
    pub objects: Vec<RestoreObjectRecord>,
}

#[derive(Debug)]
pub struct RestoreSupportRecords {
    pub editing_enabled: bool,
    pub vendors: Vec<RestoreVendorRecord>,
}

struct Token {
    value: String,
    quoted: bool,
    span: Range<usize>,
}

/// This parser preserves association rather than using the compatibility
/// reader's aggregate UUID map. Unknown layouts refuse; the integer footer is
/// opaque and is never interpreted as another supplier or object record.
pub fn restore_support_records(raw: &[u8]) -> Result<RestoreSupportRecords, String> {
    let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
    let tokens = tokens(text)?;
    let mut input = tokens.as_slice();
    if number(&mut input)? != 6 {
        return Err(invalid());
    }
    let editing_enabled = match number(&mut input)? {
        0 => true,
        1 => false,
        _ => return Err(invalid()),
    };
    let vendor_count = count(&mut input)?;
    let mut vendors = Vec::new();
    let mut payload_names = BTreeSet::new();
    for _ in 0..vendor_count {
        // These UUID slots are not asserted to be a stable vendor key or the
        // UUID of the supplier configuration. Neither meaning is established.
        uuid(&mut input)?;
        if !matches!(number(&mut input)?, 0 | 1) {
            return Err(invalid());
        }
        uuid(&mut input)?;
        let version = quoted(&mut input)?;
        let vendor = quoted(&mut input)?;
        let name = quoted(&mut input)?;
        if name.is_empty() || !payload_names.insert(name.clone()) {
            return Err(invalid());
        }
        let object_count = count(&mut input)?;
        let mut objects = Vec::new();
        let mut uuids = BTreeSet::new();
        for _ in 0..object_count {
            let rule_span = input.first().ok_or_else(invalid)?.span.clone();
            let rule = match number(&mut input)? {
                rule @ 0..=2 => rule as u8,
                _ => return Err(invalid()),
            };
            let recommendation = match number(&mut input)? {
                mark @ (-1 | 0) => mark as i8,
                _ => return Err(invalid()),
            };
            let object_uuid = uuid(&mut input)?;
            let paired_uuid = uuid(&mut input)?;
            if !uuids.insert(object_uuid.clone()) {
                return Err(invalid());
            }
            objects.push(RestoreObjectRecord {
                object_uuid,
                paired_uuid,
                rule,
                recommendation,
                rule_span,
            });
        }
        // Native blocks retain two opaque numeric slots after their records.
        // Their meaning is unknown; consuming them here preserves the next
        // supplier boundary without rewriting these bytes.
        number(&mut input)?;
        number(&mut input)?;
        vendors.push(RestoreVendorRecord {
            version,
            vendor,
            name,
            objects,
        });
    }
    // Vendor blocks are counted, not inferred by searching for a quoted name
    // or a UUID-like substring. Reject a hidden record in the remaining tail.
    if input.is_empty() {
        return Err(invalid());
    }
    while !input.is_empty() {
        number(&mut input)?;
    }
    Ok(RestoreSupportRecords {
        editing_enabled,
        vendors,
    })
}

fn invalid() -> String {
    "support restoration cannot prove supplier/object association in this marker".to_string()
}

fn take<'a>(input: &mut &'a [Token]) -> Result<&'a Token, String> {
    let (token, rest) = input.split_first().ok_or_else(invalid)?;
    *input = rest;
    Ok(token)
}

fn number(input: &mut &[Token]) -> Result<i64, String> {
    let token = take(input)?;
    if token.quoted {
        return Err(invalid());
    }
    token.value.parse().map_err(|_| invalid())
}

fn count(input: &mut &[Token]) -> Result<usize, String> {
    usize::try_from(number(input)?).map_err(|_| invalid())
}

fn uuid(input: &mut &[Token]) -> Result<String, String> {
    let token = take(input)?;
    if token.quoted || !is_uuid_text(&token.value) {
        return Err(invalid());
    }
    Ok(token.value.to_ascii_lowercase())
}

fn quoted(input: &mut &[Token]) -> Result<String, String> {
    let token = take(input)?;
    if !token.quoted {
        return Err(invalid());
    }
    Ok(token.value.clone())
}

fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let bytes = text.as_bytes();
    let mut cursor = if text.starts_with('\u{feff}') { 3 } else { 0 };
    skip_space(bytes, &mut cursor);
    if bytes.get(cursor) != Some(&b'{') {
        return Err(invalid());
    }
    cursor += 1;
    let mut tokens = Vec::new();
    loop {
        skip_space(bytes, &mut cursor);
        let mut start = cursor;
        let quoted = bytes.get(cursor) == Some(&b'"');
        let value = if quoted {
            cursor += 1;
            let mut value = String::new();
            let mut part = cursor;
            loop {
                match bytes.get(cursor) {
                    None => return Err(invalid()),
                    Some(b'"') => {
                        value.push_str(&text[part..cursor]);
                        cursor += 1;
                        if bytes.get(cursor) != Some(&b'"') {
                            break;
                        }
                        value.push('"');
                        cursor += 1;
                        part = cursor;
                    }
                    _ => cursor += 1,
                }
            }
            value
        } else {
            while bytes
                .get(cursor)
                .is_some_and(|byte| !matches!(byte, b',' | b'}'))
            {
                cursor += 1;
            }
            let value = text[start..cursor].trim();
            if value.is_empty() || value.contains(['{', '"']) {
                return Err(invalid());
            }
            value.to_string()
        };
        let end = if quoted {
            cursor
        } else {
            start += text[start..cursor].len() - text[start..cursor].trim_start().len();
            start + value.len()
        };
        skip_space(bytes, &mut cursor);
        tokens.push(Token {
            value,
            quoted,
            span: start..end,
        });
        match bytes.get(cursor) {
            Some(b',') => cursor += 1,
            Some(b'}') => {
                cursor += 1;
                skip_space(bytes, &mut cursor);
                if cursor != bytes.len() {
                    return Err(invalid());
                }
                return Ok(tokens);
            }
            _ => return Err(invalid()),
        }
    }
}

fn skip_space(bytes: &[u8], cursor: &mut usize) {
    while bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
        *cursor += 1;
    }
}

/// A narrow read port: the current side delegates to the issued proposal, so
/// tree membership, absent companion roots and every byte become plan inputs.
/// The supplier side is issued only over a private, completed conversion.
pub trait RestoreObjectRead {
    fn read(&mut self, relative: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError>;
    fn tree(&mut self, relative: &Path) -> Result<Vec<PathBuf>, ApplyStagingError>;
}

impl RestoreObjectRead for ApplyProposal<'_> {
    fn read(&mut self, relative: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        ApplyProposal::read(self, relative)
    }

    fn tree(&mut self, relative: &Path) -> Result<Vec<PathBuf>, ApplyStagingError> {
        self.enumerate_tree_with_root_absence(relative)
            .map(|(files, _)| files)
    }
}

#[derive(Debug)]
pub struct RestoreObjectImage {
    uuid: String,
    configuration: (String, String, String),
    files: std::collections::BTreeMap<PathBuf, Vec<u8>>,
}

impl RestoreObjectImage {
    pub fn uuid(&self) -> &str {
        &self.uuid
    }

    pub fn configuration_identity(&self) -> (&str, &str, &str) {
        (
            &self.configuration.0,
            &self.configuration.1,
            &self.configuration.2,
        )
    }

    /// Reports the union of both file sets; byte equality includes XML
    /// whitespace, BOM/EOL, unknown files and binary payloads.
    pub fn different_files(&self, supplier: &Self) -> Vec<PathBuf> {
        self.files
            .keys()
            .chain(supplier.files.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|path| self.files.get(*path) != supplier.files.get(*path))
            .cloned()
            .collect()
    }
}

pub fn read_restore_object_image(
    read: &mut dyn RestoreObjectRead,
    target: &MetadataAddress,
    expected_format: &str,
) -> Result<RestoreObjectImage, ApplyPlanError> {
    use unica_source_designer::owner::{
        prove_already_read_metadata_owner, prove_already_read_source_set_owner,
    };
    let invalid = || {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource,
        "support restoration requires a registered configuration object with proven UUID and complete files")
    };
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let (kind, name, form) = match parts.as_slice() {
        [kind, name] => (*kind, *name, None),
        [kind, name, "Form", form] => (*kind, *name, Some(*form)),
        _ => return Err(invalid()),
    };
    let mut required = |path: &Path| {
        read.read(path)
            .map_err(|error| ApplyPlanError::staging(error, "at"))?
            .ok_or_else(invalid)
    };
    let config_path = Path::new("Configuration.xml");
    let config_bytes = required(config_path)?;
    let config = prove_already_read_source_set_owner(
        config_path,
        &config_bytes,
        SourceSetKind::Configuration,
    )
    .map_err(|_| invalid())?;
    if config.is_configuration_extension()
        || config.version() != Some(expected_format)
        || !config.registers(kind, name)
    {
        return Err(invalid());
    }
    let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &format!("{kind}.{name}"))
        .map_err(|_| invalid())?;
    let owner_path = unica_source_designer::layout::metadata_descriptor_relative(
        &owner,
        SourceSetKind::Configuration,
    )
    .map_err(|_| invalid())?;
    let owner_bytes = required(&owner_path)?;
    let owner =
        prove_already_read_metadata_owner(&owner_path, &owner_bytes).map_err(|_| invalid())?;
    if owner.version() != Some(expected_format)
        || owner.artifact_kind() != kind
        || owner.artifact_name() != Some(name)
    {
        return Err(invalid());
    }
    let (descriptor_path, descriptor) = if let Some(form) = form {
        if !owner.registers("Form", form) {
            return Err(invalid());
        }
        let path = owner_path
            .with_extension("")
            .join("Forms")
            .join(format!("{form}.xml"));
        let bytes = required(&path)?;
        let child = prove_already_read_metadata_owner(&path, &bytes).map_err(|_| invalid())?;
        if child.version() != Some(expected_format)
            || child.artifact_kind() != "Form"
            || child.artifact_name() != Some(form)
        {
            return Err(invalid());
        }
        (path, bytes)
    } else {
        (owner_path, owner_bytes)
    };
    let text = std::str::from_utf8(&descriptor).map_err(|_| invalid())?;
    let document =
        roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|_| invalid())?;
    if document.root_element().attribute("uuid").is_some() {
        return Err(invalid());
    }
    let uuid = crate::support::support_root_uuid_from_bytes(&descriptor).ok_or_else(invalid)?;
    uuid::Uuid::parse_str(&uuid).map_err(|_| invalid())?;
    let companion = descriptor_path.with_extension("");
    let mut files = std::collections::BTreeMap::from([(descriptor_path, descriptor)]);
    for path in read
        .tree(&companion)
        .map_err(|error| ApplyPlanError::staging(error, "at"))?
    {
        if !path.starts_with(&companion) || files.contains_key(&path) {
            return Err(invalid());
        }
        let bytes = read
            .read(&path)
            .map_err(|error| ApplyPlanError::staging(error, "at"))?
            .ok_or_else(invalid)?;
        files.insert(path, bytes);
    }
    let config_text = std::str::from_utf8(&config_bytes).map_err(|_| invalid())?;
    let config_doc = roxmltree::Document::parse(config_text.trim_start_matches('\u{feff}'))
        .map_err(|_| invalid())?;
    let artifact = config_doc
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(invalid)?;
    let property = |name| {
        unica_source_designer::owner::metadata_artifact_property(artifact, name).unwrap_or_default()
    };
    let configuration = (property("Name"), property("Vendor"), property("Version"));
    Ok(RestoreObjectImage {
        uuid,
        configuration,
        files,
    })
}

/// Infrastructure owns conversion and returns an image read from its private
/// completed output. It receives the captured CF bytes, never an ambient path
/// it could reopen after the proposal's read.
pub trait RestoreSupplierRead {
    fn object_image(
        &self,
        payload: &[u8],
        target: &MetadataAddress,
        expected_format: &str,
    ) -> Result<RestoreObjectImage, ApplyPlanError>;
}

#[derive(Debug)]
pub struct RestoreSupportProof {
    pub original_marker: Vec<u8>,
    pub locked_marker: Vec<u8>,
    pub suppliers_checked: usize,
    pub files_per_supplier: usize,
    pub suppliers: Vec<unica_metadata::support::RestorationSupplier>,
}

/// Prepares only the marker's new bytes. Publication remains with the proposal
/// owner and its standard saved-plan fences; no conversion runs at execution.
pub fn prove_support_restoration(
    staged: &mut ApplyProposal<'_>,
    supplier: &dyn RestoreSupplierRead,
    target: &MetadataAddress,
    expected_format: &str,
) -> Result<RestoreSupportProof, ApplyPlanError> {
    let invalid_state = |message| ApplyPlanError::new(ApplyPlanErrorKind::InvalidState, message);
    let marker_path = Path::new("Ext/ParentConfigurations.bin");
    let original_marker = staged
        .read(marker_path)
        .map_err(|error| ApplyPlanError::staging(error, "at"))?
        .ok_or_else(|| invalid_state("the object has no vendor support to restore".to_string()))?;
    let records = restore_support_records(&original_marker)
        .map_err(|message| ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message))?;
    let current = read_restore_object_image(staged, target, expected_format)?;
    let vendors = records
        .vendors
        .iter()
        .filter_map(|vendor| {
            vendor
                .objects
                .iter()
                .find(|record| record.object_uuid.eq_ignore_ascii_case(current.uuid()))
                .map(|record| (vendor, record))
        })
        .collect::<Vec<_>>();
    if vendors.is_empty() {
        return Err(invalid_state(
            "the object's UUID has no supplier record to restore".to_string(),
        ));
    }
    let payload_root = Path::new("Ext/ParentConfigurations");
    let payload_files = staged
        .enumerate_tree_with_root_absence(payload_root)
        .map_err(|error| ApplyPlanError::staging(error, "at"))?
        .0;
    let mut spans = Vec::new();
    for (vendor, record) in &vendors {
        staged
            .checkpoint("supplier support comparison")
            .map_err(|error| ApplyPlanError::staging(error, "at"))?;
        // Names originate in an untrusted marker, and must be one portable
        // filename component before joining the retained payload directory.
        if vendor.name.contains(['/', '\\', ':']) || matches!(vendor.name.as_str(), "." | "..") {
            return Err(invalid_state(
                "the supplier payload name is not one filename component".to_string(),
            ));
        }
        let filename = format!("{}.cf", vendor.name);
        let candidates = payload_files
            .iter()
            .filter(|path| {
                path.parent() == Some(payload_root)
                    && path.file_stem().and_then(|stem| stem.to_str()) == Some(vendor.name.as_str())
                    && path
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("cf"))
            })
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(invalid_state(format!(
                "support restoration needs exactly one supplier payload {filename}"
            )));
        }
        let payload = staged
            .read(candidates[0])
            .map_err(|error| ApplyPlanError::staging(error, "at"))?
            .ok_or_else(|| {
                invalid_state("the supplier payload disappeared during comparison".to_string())
            })?;
        let image = supplier.object_image(&payload, target, expected_format)?;
        if image.configuration_identity() != (&vendor.name, &vendor.vendor, &vendor.version)
            || !image.uuid().eq_ignore_ascii_case(&record.paired_uuid)
        {
            return Err(invalid_state(
                "the CF's configuration or object identity does not match its supplier record"
                    .to_string(),
            ));
        }
        let different = current.different_files(&image);
        if let Some(first) = different.first() {
            use unica_source_contract::portable_path::PortablePathExt;
            return Err(invalid_state(format!("support restoration requires byte-for-byte equality with every supplier; {} file(s) differ, starting with {}", different.len(), first.portable())));
        }
        spans.push(record.rule_span.clone());
    }
    let mut text =
        String::from_utf8(original_marker.clone()).expect("the marker parser validated UTF-8");
    spans.sort_by_key(|span| std::cmp::Reverse(span.start));
    for span in spans {
        if &text[span.clone()] != "0" {
            text.replace_range(span, "0");
        }
    }
    Ok(RestoreSupportProof {
        original_marker,
        locked_marker: text.into_bytes(),
        suppliers_checked: vendors.len(),
        files_per_supplier: current.files.len(),
        suppliers: vendors
            .iter()
            .map(|(vendor, _)| unica_metadata::support::RestorationSupplier {
                name: vendor.name.clone(),
                vendor: vendor.vendor.clone(),
                version: vendor.version.clone(),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use unica_source_contract::proposal::{ProposalBinding, SourceProposal, StagedApplyChange};

    const EDITABLE: &[u8] =
        include_bytes!("../../../tests/fixtures/platform_8_3_27/support-markers/editable.bin");
    const LOCKED: &[u8] =
        include_bytes!("../../../tests/fixtures/platform_8_3_27/support-markers/locked.bin");
    const OFF: &[u8] =
        include_bytes!("../../../tests/fixtures/platform_8_3_27/support-markers/off.bin");

    #[test]
    fn native_markers_retain_supplier_records_and_original_rule_spans() {
        for (raw, enabled, rule) in [(EDITABLE, true, 1), (LOCKED, true, 0), (OFF, false, 1)] {
            let parsed = restore_support_records(raw).unwrap();
            assert_eq!(parsed.editing_enabled, enabled);
            assert_eq!(parsed.vendors.len(), 1);
            let vendor = &parsed.vendors[0];
            assert_eq!(
                (&*vendor.version, &*vendor.vendor, &*vendor.name),
                ("1.0.0.1", "Unica test vendor", "CorpusConfiguration")
            );
            assert_eq!(vendor.objects.len(), 5);
            for object in &vendor.objects {
                assert_eq!(object.rule, rule);
                assert_eq!(object.recommendation, 0);
                assert_eq!(object.object_uuid, object.paired_uuid);
                assert_eq!(&raw[object.rule_span.clone()], rule.to_string().as_bytes());
            }
        }
    }

    #[test]
    fn native_three_supplier_marker_preserves_each_block_and_rule_span() {
        let raw = include_bytes!(
            "../../../tests/fixtures/support-restore-multi-vendor/ParentConfigurations.bin"
        );
        let parsed = restore_support_records(raw).unwrap();
        assert_eq!(parsed.vendors.len(), 3);
        for (vendor, name, count) in [
            (&parsed.vendors[0], "Поставка1", 56),
            (&parsed.vendors[1], "Поставка2", 8),
            (&parsed.vendors[2], "Поставка3", 6),
        ] {
            assert_eq!(vendor.name, name);
            assert_eq!(vendor.objects.len(), count);
            for object in &vendor.objects {
                assert_eq!(
                    &raw[object.rule_span.clone()],
                    object.rule.to_string().as_bytes()
                );
            }
        }
        assert_eq!(parsed.vendors[0].version, "1.0.0.1");
        assert_eq!(parsed.vendors[1].version, "2.0.1");
        assert_eq!(parsed.vendors[2].vendor, "\"Обычный Поставщик\"");
        for vendor in &parsed.vendors[1..] {
            assert_eq!(
                vendor
                    .objects
                    .iter()
                    .filter(|object| object.object_uuid != object.paired_uuid)
                    .count(),
                2
            );
        }
        // Editing rules in later blocks must preserve all supplier metadata,
        // opaque slots, and the complete numeric tail byte for byte.
        let mut changed = raw.to_vec();
        let spans = parsed
            .vendors
            .iter()
            .flat_map(|vendor| &vendor.objects)
            .map(|object| object.rule_span.clone())
            .collect::<Vec<_>>();
        for span in &spans {
            assert_eq!(span.len(), 1);
            changed[span.start] = b'0';
        }
        for (index, (&before, &after)) in raw.iter().zip(&changed).enumerate() {
            if !spans.iter().any(|span| span.contains(&index)) {
                assert_eq!(before, after);
            }
        }
        let reparsed = restore_support_records(&changed).unwrap();
        assert!(reparsed
            .vendors
            .iter()
            .flat_map(|vendor| &vendor.objects)
            .all(|object| object.rule == 0));
    }

    #[test]
    fn damaged_block_counts_and_unassociated_records_refuse() {
        let original = std::str::from_utf8(EDITABLE).unwrap();
        for text in [
            original.replacen(",5,1,0,", ",4,1,0,", 1),
            original.replacen(",5,1,0,", ",6,1,0,", 1),
            original.replacen("{6,0,1,", "{6,0,2,", 1),
            original.replace(
                "00000000-0000-0000-0000-000000000001",
                "bf5222b1-a591-40d2-aabf-ea9a296792d6",
            ),
            original.replacen(",1,0,bf522", ",1,-2,bf522", 1),
        ] {
            assert!(restore_support_records(text.as_bytes()).is_err());
        }
        for end in 0..EDITABLE.len() {
            if EDITABLE[end..].iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            assert!(
                restore_support_records(&EDITABLE[..end]).is_err(),
                "prefix {end}"
            );
        }
    }

    #[test]
    fn synthetic_supplier_blocks_do_not_flatten_rules_or_uuid_pairs() {
        // Synthetic boundary/escaping negative fixture, not evidence of the
        // platform's multi-supplier format or the second UUID's meaning.
        let id = "11111111-1111-1111-1111-111111111111";
        let paired = "22222222-2222-2222-2222-222222222222";
        let block = |name: &str, rule: u8, mark: i8| {
            format!("{id},0,{id},\"1.0\",\"Vendor, \"\"quoted\"\"\",\"{name}\",1,{rule},{mark},{id},{paired},0,0")
        };
        let text = format!(
            "\u{feff}{{6,0,2,{},{},0,1}}\r\n",
            block("Первый", 1, -1),
            block("Второй", 0, 0)
        );
        let parsed = restore_support_records(text.as_bytes()).unwrap();
        assert_eq!(parsed.vendors.len(), 2);
        for (vendor, name, rule, recommendation) in [
            (&parsed.vendors[0], "Первый", 1, -1),
            (&parsed.vendors[1], "Второй", 0, 0),
        ] {
            assert_eq!(vendor.name, name);
            assert_eq!(vendor.vendor, "Vendor, \"quoted\"");
            let object = &vendor.objects[0];
            assert_eq!(object.object_uuid, id);
            assert_eq!(object.paired_uuid, paired);
            assert_eq!(object.rule, rule);
            assert_eq!(object.recommendation, recommendation);
            assert_eq!(&text[object.rule_span.clone()], rule.to_string());
        }
        // Both opaque slots belong to each block. Missing or nonnumeric
        // slots must not shift the following supplier into the current one.
        for replacement in ["", ",0", ",0,\"unknown\""] {
            let broken = text.replacen(
                &format!("{paired},0,0,"),
                &format!("{paired}{replacement},"),
                1,
            );
            assert!(restore_support_records(broken.as_bytes()).is_err());
        }
        let nonzero_opaque =
            text.replacen(&format!("{paired},0,0,"), &format!("{paired},7,-3,"), 1);
        assert_eq!(
            restore_support_records(nonzero_opaque.as_bytes())
                .unwrap()
                .vendors
                .len(),
            2
        );
        let duplicate = text.replace("Второй", "Первый");
        assert!(restore_support_records(duplicate.as_bytes()).is_err());
        let hidden = text.replace(",0,1}", &format!(",0,0,{id},{paired}}}"));
        assert!(restore_support_records(hidden.as_bytes()).is_err());
    }

    #[test]
    fn original_rule_span_excludes_surrounding_whitespace() {
        let native = std::str::from_utf8(EDITABLE).unwrap();
        let text = native.replacen(",5,1,0,", ",5, \t1 \t,0,", 1);
        let parsed = restore_support_records(text.as_bytes()).unwrap();
        let span = parsed.vendors[0].objects[0].rule_span.clone();
        assert_eq!(&text[span.clone()], "1");
        let mut changed = text.clone();
        changed.replace_range(span, "0");
        assert_eq!(changed, text.replacen(",5, \t1 \t,0,", ",5, \t0 \t,0,", 1));
    }

    #[derive(Clone, Default)]
    struct ObjectRead {
        files: BTreeMap<PathBuf, Vec<u8>>,
        reads: BTreeSet<PathBuf>,
        trees: Vec<PathBuf>,
    }

    impl RestoreObjectRead for ObjectRead {
        fn read(&mut self, path: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError> {
            self.reads.insert(path.to_path_buf());
            Ok(self.files.get(path).cloned())
        }
        fn tree(&mut self, path: &Path) -> Result<Vec<PathBuf>, ApplyStagingError> {
            self.trees.push(path.to_path_buf());
            Ok(self
                .files
                .keys()
                .filter(|file| file.starts_with(path))
                .cloned()
                .collect())
        }
    }

    impl SourceProposal for ObjectRead {
        fn checkpoint(&self, _: &str) -> Result<(), ApplyStagingError> {
            Ok(())
        }
        fn binding(&self) -> ProposalBinding {
            ProposalBinding::from_observation(
                unica_source_contract::read::FileIdentity::from_observation(1, 1),
                1,
            )
        }
        fn enumerate_tree(&mut self, path: &Path) -> Result<Vec<PathBuf>, ApplyStagingError> {
            RestoreObjectRead::tree(self, path)
        }
        fn enumerate_tree_with_root_absence(
            &mut self,
            path: &Path,
        ) -> Result<(Vec<PathBuf>, bool), ApplyStagingError> {
            let files = RestoreObjectRead::tree(self, path)?;
            let absent = files.is_empty();
            Ok((files, absent))
        }
        fn read(&mut self, path: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError> {
            RestoreObjectRead::read(self, path)
        }
        fn retained_relative_path(&mut self, path: &Path) -> Result<PathBuf, ApplyStagingError> {
            Ok(path.to_path_buf())
        }
        fn require_exact_path_spelling(&mut self, _: &Path) -> Result<(), ApplyStagingError> {
            Ok(())
        }
        fn read_bounded(
            &mut self,
            path: &Path,
            limit: usize,
        ) -> Result<Option<Vec<u8>>, ApplyStagingError> {
            let bytes = RestoreObjectRead::read(self, path)?;
            assert!(bytes.as_ref().is_none_or(|bytes| bytes.len() <= limit));
            Ok(bytes)
        }
        fn original_image(&self, path: &Path) -> Option<&[u8]> {
            self.files.get(path).map(Vec::as_slice)
        }
        fn read_guarded_bounded(
            &mut self,
            path: &Path,
            limit: usize,
        ) -> Result<Option<Vec<u8>>, ApplyStagingError> {
            SourceProposal::read_bounded(self, path, limit)
        }
        fn create(&mut self, _: &Path, _: Vec<u8>) -> Result<(), ApplyStagingError> {
            panic!("comparison must not mutate the proposal")
        }
        fn replace(&mut self, _: &Path, _: &[u8], _: Vec<u8>) -> Result<(), ApplyStagingError> {
            panic!("comparison must not mutate the proposal")
        }
        fn remove(&mut self, _: &Path, _: &[u8]) -> Result<(), ApplyStagingError> {
            panic!("comparison must not mutate the proposal")
        }
        fn remove_owned_directories(
            &mut self,
            _: &Path,
            _: &Path,
            _: &Path,
        ) -> Result<(), ApplyStagingError> {
            panic!("comparison must not mutate the proposal")
        }
        fn planned_changes(&self) -> Vec<StagedApplyChange> {
            Vec::new()
        }
    }

    struct SupplierRead {
        calls: std::cell::RefCell<Vec<Vec<u8>>>,
        differing_second: bool,
    }

    impl RestoreSupplierRead for SupplierRead {
        fn object_image(
            &self,
            payload: &[u8],
            target: &MetadataAddress,
            format: &str,
        ) -> Result<RestoreObjectImage, ApplyPlanError> {
            self.calls.borrow_mut().push(payload.to_vec());
            let mut fixture = object_fixture();
            if payload == b"second test supplier" {
                let bytes = fixture
                    .files
                    .get_mut(Path::new("Configuration.xml"))
                    .unwrap();
                *bytes = std::str::from_utf8(bytes)
                    .unwrap()
                    .replacen(
                        "<Name>РеактивныйHTML</Name>",
                        "<Name>SecondSupplier</Name>",
                        1,
                    )
                    .into_bytes();
                if self.differing_second {
                    fixture
                        .files
                        .get_mut(Path::new(
                            "DataProcessors/ТестHTML/Forms/Форма/Ext/Form/Module.bsl",
                        ))
                        .unwrap()
                        .push(b'\n');
                }
            } else {
                assert_eq!(payload, b"first test supplier");
            }
            read_restore_object_image(&mut fixture, target, format)
        }
    }

    fn object_fixture() -> ObjectRead {
        let mut read = ObjectRead::default();
        macro_rules! file {
            ($path:literal) => {
                read.files.insert(
                    PathBuf::from($path),
                    include_bytes!(concat!(
                        "../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/",
                        $path
                    ))
                    .to_vec(),
                );
            };
        }
        file!("Configuration.xml");
        file!("DataProcessors/ТестHTML.xml");
        file!("DataProcessors/ТестHTML/Forms/Форма.xml");
        file!("DataProcessors/ТестHTML/Forms/Форма/Ext/Form.xml");
        file!("DataProcessors/ТестHTML/Forms/Форма/Ext/Form/Module.bsl");
        file!("DataProcessors/ТестHTML/Templates/МакетСобранныйHTML.xml");
        file!("DataProcessors/ТестHTML/Templates/МакетСобранныйHTML/Ext/Template.bin");
        read
    }

    fn target() -> MetadataAddress {
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "DataProcessor.ТестHTML").unwrap()
    }

    #[test]
    fn full_native_object_includes_forms_modules_and_binary_templates() {
        let mut read = object_fixture();
        let unrelated = PathBuf::from("CommonModules/Other.xml");
        read.files.insert(unrelated.clone(), b"unrelated".to_vec());
        let image = read_restore_object_image(&mut read, &target(), "2.20").unwrap();
        assert_eq!(image.files.len(), 6);
        assert!(!read.reads.contains(&unrelated));
        assert_eq!(read.trees, [PathBuf::from("DataProcessors/ТестHTML")]);
        let supplier = read_restore_object_image(&mut object_fixture(), &target(), "2.20").unwrap();
        assert!(image.different_files(&supplier).is_empty());
        for path in image.files.keys() {
            let mut changed = object_fixture();
            // The descriptor and form remain valid XML; even a formatting-only
            // byte addition prevents the requested exact restoration.
            changed
                .files
                .get_mut(path)
                .unwrap()
                .extend_from_slice(b"\n");
            let changed = read_restore_object_image(&mut changed, &target(), "2.20").unwrap();
            assert_eq!(
                changed.different_files(&supplier),
                std::slice::from_ref(path)
            );
        }
    }

    #[test]
    fn file_set_union_includes_unknown_files_and_detects_missing_payloads() {
        let mut read = object_fixture();
        let unknown = PathBuf::from("DataProcessors/ТестHTML/Ext/future-payload.opaque");
        read.files.insert(unknown.clone(), vec![0, 255, 128]);
        let image = read_restore_object_image(&mut read, &target(), "2.20").unwrap();
        let supplier = read_restore_object_image(&mut object_fixture(), &target(), "2.20").unwrap();
        assert_eq!(
            image.different_files(&supplier),
            std::slice::from_ref(&unknown)
        );
        assert_eq!(supplier.different_files(&image), [unknown]);
        let mut missing = object_fixture();
        let binary =
            PathBuf::from("DataProcessors/ТестHTML/Templates/МакетСобранныйHTML/Ext/Template.bin");
        missing.files.remove(&binary);
        let missing = read_restore_object_image(&mut missing, &target(), "2.20").unwrap();
        assert_eq!(missing.different_files(&supplier), [binary]);
    }

    #[test]
    fn registered_form_captures_only_its_complete_companion_tree() {
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "DataProcessor.ТестHTML.Form.Форма",
        )
        .unwrap();
        let mut read = object_fixture();
        let image = read_restore_object_image(&mut read, &target, "2.20").unwrap();
        assert_eq!(image.files.len(), 3);
        assert_eq!(
            read.trees,
            [PathBuf::from("DataProcessors/ТестHTML/Forms/Форма")]
        );
        assert!(read
            .reads
            .contains(Path::new("DataProcessors/ТестHTML.xml")));
        assert!(!image
            .files
            .contains_key(Path::new("DataProcessors/ТестHTML.xml")));
        assert!(!read.reads.contains(Path::new(
            "DataProcessors/ТестHTML/Templates/МакетСобранныйHTML/Ext/Template.bin"
        )));
    }

    #[test]
    fn comparison_cannot_admit_an_unregistered_or_wrong_identity_object() {
        let descriptor = Path::new("DataProcessors/ТестHTML.xml");
        for (path, from, to) in [
            (descriptor, "<Name>ТестHTML</Name>", "<Name>Другой</Name>"),
            (
                Path::new("Configuration.xml"),
                "<DataProcessor>ТестHTML</DataProcessor>",
                "<DataProcessor>Другой</DataProcessor>",
            ),
            (
                descriptor,
                "<MetaDataObject ",
                "<MetaDataObject uuid=\"11111111-1111-1111-1111-111111111111\" ",
            ),
            (descriptor, "http://v8.1c.ru/8.3/MDClasses", "urn:foreign"),
            (Path::new("Configuration.xml"), "<Properties>", "<Properties><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>"),
        ] {
            let mut read = object_fixture();
            let bytes = read.files.get_mut(path).unwrap();
            let text = std::str::from_utf8(bytes).unwrap();
            assert!(text.contains(from));
            *bytes = text.replacen(from, to, 1).into_bytes();
            assert!(read_restore_object_image(&mut read, &target(), "2.20").is_err());
        }
    }

    fn supported_fixture(two_suppliers: bool, already_locked: bool) -> ObjectRead {
        // A synthetic association/proof test. Native two-vendor provenance is
        // a separate acceptance requirement before exposing restoration.
        let mut read = object_fixture();
        let image = read_restore_object_image(&mut read, &target(), "2.20").unwrap();
        let id = image.uuid();
        let rule = if already_locked { 0 } else { 1 };
        let (_, vendor, version) = image.configuration_identity();
        let block = |name: &str| {
            format!("{id},0,{id},\"{version}\",\"{vendor}\",\"{name}\",1,{rule},0,{id},{id},0,0")
        };
        let marker = if two_suppliers {
            format!(
                "{{6,0,2,{},{},0,0,1}}",
                block("РеактивныйHTML"),
                block("SecondSupplier")
            )
        } else {
            format!("{{6,0,1,{},0,0,1}}", block("РеактивныйHTML"))
        };
        read.files.insert(
            PathBuf::from("Ext/ParentConfigurations.bin"),
            marker.into_bytes(),
        );
        read.files.insert(
            PathBuf::from("Ext/ParentConfigurations/РеактивныйHTML.cf"),
            b"first test supplier".to_vec(),
        );
        if two_suppliers {
            read.files.insert(
                PathBuf::from("Ext/ParentConfigurations/SecondSupplier.cf"),
                b"second test supplier".to_vec(),
            );
        }
        read
    }

    #[test]
    fn restoration_checks_every_supplier_before_preparing_only_rule_bytes() {
        let mut read = supported_fixture(true, false);
        let original = read.files.clone();
        let supplier = SupplierRead {
            calls: Default::default(),
            differing_second: false,
        };
        let proof = prove_support_restoration(
            &mut ApplyProposal::from_port(&mut read),
            &supplier,
            &target(),
            "2.20",
        )
        .unwrap();
        assert_eq!(proof.suppliers_checked, 2);
        assert_eq!(proof.files_per_supplier, 6);
        assert_eq!(
            supplier.calls.borrow().as_slice(),
            [
                b"first test supplier".to_vec(),
                b"second test supplier".to_vec()
            ]
        );
        let original_records = restore_support_records(&proof.original_marker).unwrap();
        let changed = proof
            .original_marker
            .iter()
            .zip(&proof.locked_marker)
            .enumerate()
            .filter_map(|(offset, (old, new))| (old != new).then_some(offset))
            .collect::<Vec<_>>();
        assert_eq!(
            changed,
            original_records
                .vendors
                .iter()
                .map(|vendor| vendor.objects[0].rule_span.start)
                .collect::<Vec<_>>()
        );
        assert!(restore_support_records(&proof.locked_marker)
            .unwrap()
            .vendors
            .iter()
            .all(|vendor| vendor.objects[0].rule == 0));
        assert_eq!(read.files, original);
        assert!(read
            .reads
            .contains(Path::new("Ext/ParentConfigurations/SecondSupplier.cf")));
        assert!(read
            .trees
            .contains(&PathBuf::from("Ext/ParentConfigurations")));
    }

    #[test]
    fn one_matching_supplier_cannot_authorize_restore_against_a_different_second() {
        let mut read = supported_fixture(true, false);
        let original = read.files.clone();
        let supplier = SupplierRead {
            calls: Default::default(),
            differing_second: true,
        };
        let error = prove_support_restoration(
            &mut ApplyProposal::from_port(&mut read),
            &supplier,
            &target(),
            "2.20",
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        assert!(error.to_string().contains("Module.bsl"));
        assert_eq!(supplier.calls.borrow().len(), 2);
        assert_eq!(read.files, original);
    }

    #[test]
    fn already_locked_restoration_still_requires_comparison() {
        let mut read = supported_fixture(false, true);
        let supplier = SupplierRead {
            calls: Default::default(),
            differing_second: false,
        };
        let proof = prove_support_restoration(
            &mut ApplyProposal::from_port(&mut read),
            &supplier,
            &target(),
            "2.20",
        )
        .unwrap();
        assert_eq!(proof.locked_marker, proof.original_marker);
        assert_eq!(supplier.calls.borrow().len(), 1);
        let bytes = read
            .files
            .get_mut(Path::new(
                "DataProcessors/ТестHTML/Forms/Форма/Ext/Form/Module.bsl",
            ))
            .unwrap();
        bytes.push(b'\n');
        assert!(prove_support_restoration(
            &mut ApplyProposal::from_port(&mut read),
            &supplier,
            &target(),
            "2.20"
        )
        .is_err());
    }
}
