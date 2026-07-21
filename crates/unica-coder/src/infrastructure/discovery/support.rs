use crate::application::discovery::ports::SupportStatePort;
use crate::domain::discovery::{
    DiscoveryQuery, EvidenceLocation, FactBatch, ProviderDiagnostic, ProviderOutcome, SourceFile,
    SourceInventory, SupportFact, SupportStateKind,
};
use crate::infrastructure::discovery::metadata::{
    analyzed_file_map, build_batch, contributors_for_records, inventory_is_bounded,
    parse_inventory_descriptors, MetadataDescriptor,
};
use crate::infrastructure::native_operations::common::{
    parse_support_state_bytes, ParsedSupportState, SupportObjectRule,
};
use std::collections::BTreeSet;

pub(crate) struct SupportStateProvider;

impl SupportStatePort for SupportStateProvider {
    fn support(
        &self,
        query: &DiscoveryQuery<'_>,
        files: &SourceInventory,
    ) -> ProviderOutcome<FactBatch<SupportFact>> {
        match collect_support_facts(query, files) {
            Ok(SupportCollection::Complete(batch)) => ProviderOutcome::Complete(batch),
            Ok(SupportCollection::Bounded { batch, diagnostic }) => ProviderOutcome::Bounded {
                data: batch,
                diagnostic,
            },
            Err(diagnostic) => ProviderOutcome::ContractViolation(diagnostic),
        }
    }
}

enum SupportCollection {
    Complete(FactBatch<SupportFact>),
    Bounded {
        batch: FactBatch<SupportFact>,
        diagnostic: ProviderDiagnostic,
    },
}

fn collect_support_facts(
    query: &DiscoveryQuery<'_>,
    inventory: &SourceInventory,
) -> Result<SupportCollection, ProviderDiagnostic> {
    let descriptors = parse_inventory_descriptors(inventory)?;
    let inventory_bounded = inventory_is_bounded(inventory);
    validate_unique_support_artifacts(&descriptors)?;
    let mut analyzed_files = analyzed_file_map(&descriptors);
    let support_file = root_support_file(inventory)?;
    if let Some(file) = support_file {
        analyzed_files.insert(file.relative_path.clone(), file.analyzed_file());
    }
    let parsed = match support_file {
        Some(file) => Some(parse_support_state_bytes(&file.bytes).map_err(|error| {
            ProviderDiagnostic::material(
                "support_state_malformed",
                format!(
                    "support state {} is malformed: {error}",
                    file.relative_path.as_str()
                ),
            )
        })?),
        None => None,
    };

    let mut records = if support_file.is_none() && inventory_bounded {
        Vec::new()
    } else {
        descriptors
            .iter()
            .map(|descriptor| support_fact(descriptor, support_file, parsed.as_ref()))
            .collect::<Result<Vec<_>, _>>()?
    };
    records.sort();
    let bounded = records.len() > usize::from(query.limits().max_evidence);
    if bounded {
        records.truncate(usize::from(query.limits().max_evidence));
    }
    let analyzed_files = analyzed_files.into_values().collect::<Vec<_>>();
    let contributors = contributors_for_records(&records, &analyzed_files);
    let batch = build_batch(records, analyzed_files, contributors)?;
    if bounded {
        Ok(SupportCollection::Bounded {
            batch,
            diagnostic: ProviderDiagnostic::material(
                "support_state_evidence_bound",
                "support-state facts stopped at the maxEvidence limit",
            ),
        })
    } else if inventory_bounded {
        Ok(SupportCollection::Bounded {
            batch,
            diagnostic: ProviderDiagnostic::material(
                "support_state_inventory_bounded",
                "support-state scope is incomplete because source inventory was truncated",
            ),
        })
    } else {
        Ok(SupportCollection::Complete(batch))
    }
}

fn validate_unique_support_artifacts(
    descriptors: &[MetadataDescriptor],
) -> Result<(), ProviderDiagnostic> {
    let mut artifacts = BTreeSet::new();
    for descriptor in descriptors {
        if !artifacts.insert(descriptor.root.artifact.clone()) {
            return Err(ProviderDiagnostic::material(
                "support_artifact_identity_conflict",
                "metadata descriptors contain duplicate canonical root identities",
            ));
        }
    }
    Ok(())
}

fn root_support_file(
    inventory: &SourceInventory,
) -> Result<Option<&SourceFile>, ProviderDiagnostic> {
    let mut matches = inventory.files.iter().filter(|file| {
        let mut components = file.relative_path.as_str().split('/');
        matches!(
            (components.next(), components.next(), components.next()),
            (Some(ext), Some(name), None)
                if ext.eq_ignore_ascii_case("Ext")
                    && name.eq_ignore_ascii_case("ParentConfigurations.bin")
        )
    });
    let first = matches.next();
    if matches.next().is_some() {
        return Err(ProviderDiagnostic::material(
            "support_state_source_conflict",
            "source inventory contains more than one root support-state file",
        ));
    }
    Ok(first)
}

fn support_fact(
    descriptor: &MetadataDescriptor,
    support_file: Option<&SourceFile>,
    parsed: Option<&ParsedSupportState>,
) -> Result<SupportFact, ProviderDiagnostic> {
    let (state, location) = match (support_file, parsed) {
        (None, None) => (
            SupportStateKind::NotOnSupport,
            descriptor.root.location.clone(),
        ),
        (Some(file), Some(state)) => {
            let object_rule = descriptor
                .root
                .object_uuid
                .as_deref()
                .and_then(|uuid| state.object_rule(uuid));
            let support_state = if state.removed() {
                SupportStateKind::Removed
            } else if !state.global_editing_enabled() {
                SupportStateKind::Locked
            } else {
                match object_rule {
                    Some(SupportObjectRule::Locked) => SupportStateKind::Locked,
                    Some(SupportObjectRule::Editable) => SupportStateKind::Editable,
                    Some(SupportObjectRule::OffSupport) | None => SupportStateKind::NotOnSupport,
                }
            };
            let line = match descriptor.root.object_uuid.as_deref() {
                Some(uuid) => support_rule_line(&file.bytes, uuid)?,
                None => 1,
            };
            (
                support_state,
                EvidenceLocation {
                    relative_path: file.relative_path.clone(),
                    line: Some(line),
                    column: None,
                    xml_path: None,
                },
            )
        }
        (None, Some(_state)) => {
            return Err(ProviderDiagnostic::material(
                "support_state_internal_contract",
                "support file and parsed state presence diverged",
            ));
        }
        (Some(_file), None) => {
            return Err(ProviderDiagnostic::material(
                "support_state_internal_contract",
                "support file and parsed state presence diverged",
            ));
        }
    };
    Ok(SupportFact {
        artifact: descriptor.root.artifact.clone(),
        state,
        location,
    })
}

fn support_rule_line(bytes: &[u8], object_uuid: &str) -> Result<u32, ProviderDiagnostic> {
    let content = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(content) => content,
        None => bytes,
    };
    let text = std::str::from_utf8(content).map_err(|_error| {
        ProviderDiagnostic::material(
            "support_state_malformed",
            "support state is not valid UTF-8",
        )
    })?;
    let Some(offset) = text
        .to_ascii_lowercase()
        .find(&object_uuid.to_ascii_lowercase())
    else {
        return Ok(1);
    };
    let line_count = text[..offset].bytes().filter(|byte| *byte == b'\n').count();
    u32::try_from(line_count.saturating_add(1)).map_err(|_error| {
        ProviderDiagnostic::material(
            "support_state_location_overflow",
            "support-state line number overflowed",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::SupportStateProvider;
    use crate::application::discovery::ports::SupportStatePort;
    use crate::domain::discovery::{
        ArtifactId, ContentHash, DiscoveryQuery, DiscoveryQueryLimits, PortableRelativePath,
        ProviderCoverage, ProviderOutcome, SourceFile, SourceInventory, SupportStateKind,
    };

    const LOCKED_UUID: &str = "40000000-0000-0000-0000-000000000001";
    const EDITABLE_UUID: &str = "40000000-0000-0000-0000-000000000002";

    #[test]
    fn absence_of_support_bytes_is_typed_not_on_support() {
        let inventory = inventory(vec![descriptor(
            "Documents/Purchase.xml",
            "Document",
            "Purchase",
            LOCKED_UUID,
        )]);

        let outcome = SupportStateProvider.support(&query(100), &inventory);

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("expected complete support facts");
        };
        assert_eq!(
            state_for(&batch.records, "Document.Purchase"),
            Some(SupportStateKind::NotOnSupport)
        );
        assert_eq!(batch.analyzed_files.len(), 1);
        assert_eq!(batch.contributors, batch.analyzed_files);
    }

    #[test]
    fn global_lock_and_typed_object_rules_map_to_support_states() {
        let descriptors = vec![
            descriptor("Documents/Locked.xml", "Document", "Locked", LOCKED_UUID),
            descriptor(
                "Documents/Editable.xml",
                "Document",
                "Editable",
                EDITABLE_UUID,
            ),
        ];
        let locked_bytes = support_bytes(1, &[(LOCKED_UUID, 1), (EDITABLE_UUID, 2)]);
        let mut locked_inventory_files = descriptors.clone();
        locked_inventory_files.push(source_file("Ext/ParentConfigurations.bin", &locked_bytes));

        let locked = SupportStateProvider.support(&query(100), &inventory(locked_inventory_files));

        let ProviderOutcome::Complete(locked) = locked else {
            panic!("expected locked support facts");
        };
        assert_eq!(
            state_for(&locked.records, "Document.Locked"),
            Some(SupportStateKind::Locked)
        );
        assert_eq!(
            state_for(&locked.records, "Document.Editable"),
            Some(SupportStateKind::Locked)
        );

        let editable_bytes = support_bytes(0, &[(LOCKED_UUID, 0), (EDITABLE_UUID, 1)]);
        let mut editable_inventory_files = descriptors;
        editable_inventory_files.push(source_file("Ext/ParentConfigurations.bin", &editable_bytes));
        let editable =
            SupportStateProvider.support(&query(100), &inventory(editable_inventory_files));

        let ProviderOutcome::Complete(editable) = editable else {
            panic!("expected object support facts");
        };
        assert_eq!(
            state_for(&editable.records, "Document.Locked"),
            Some(SupportStateKind::Locked)
        );
        assert_eq!(
            state_for(&editable.records, "Document.Editable"),
            Some(SupportStateKind::Editable)
        );

        let off_support_bytes = support_bytes(0, &[(LOCKED_UUID, 0), (EDITABLE_UUID, 2)]);
        let off_support_inventory = inventory(vec![
            descriptor("Documents/Locked.xml", "Document", "Locked", LOCKED_UUID),
            descriptor(
                "Documents/Editable.xml",
                "Document",
                "Editable",
                EDITABLE_UUID,
            ),
            source_file("Ext/ParentConfigurations.bin", &off_support_bytes),
        ]);
        let off_support = SupportStateProvider.support(&query(100), &off_support_inventory);
        let ProviderOutcome::Complete(off_support) = off_support else {
            panic!("expected off-support object fact");
        };
        assert_eq!(
            state_for(&off_support.records, "Document.Editable"),
            Some(SupportStateKind::NotOnSupport)
        );
    }

    #[test]
    fn removed_and_malformed_support_inputs_are_distinct_typed_outcomes() {
        let descriptor = descriptor(
            "Documents/Purchase.xml",
            "Document",
            "Purchase",
            LOCKED_UUID,
        );
        let removed = inventory(vec![
            descriptor.clone(),
            source_file("Ext/ParentConfigurations.bin", b"removed"),
        ]);
        let malformed = inventory(vec![
            descriptor,
            source_file(
                "Ext/ParentConfigurations.bin",
                b"this input is longer than thirty-two bytes but has no support header",
            ),
        ]);

        let removed_outcome = SupportStateProvider.support(&query(100), &removed);
        let malformed_outcome = SupportStateProvider.support(&query(100), &malformed);

        let ProviderOutcome::Complete(removed_batch) = removed_outcome else {
            panic!("short support payload is the established removed marker");
        };
        assert_eq!(
            state_for(&removed_batch.records, "Document.Purchase"),
            Some(SupportStateKind::Removed)
        );
        let ProviderOutcome::ContractViolation(diagnostic) = malformed_outcome else {
            panic!("malformed support input must invalidate the whole provider");
        };
        assert_eq!(diagnostic.code, "support_state_malformed");
    }

    #[test]
    fn evidence_bound_returns_a_typed_bounded_prefix_with_exact_coverage() {
        let bytes = support_bytes(0, &[(LOCKED_UUID, 0), (EDITABLE_UUID, 1)]);
        let inventory = inventory(vec![
            descriptor("Documents/Locked.xml", "Document", "Locked", LOCKED_UUID),
            descriptor(
                "Documents/Editable.xml",
                "Document",
                "Editable",
                EDITABLE_UUID,
            ),
            source_file("Ext/ParentConfigurations.bin", &bytes),
        ]);

        let outcome = SupportStateProvider.support(&query(1), &inventory);

        let ProviderOutcome::Bounded { data, diagnostic } = outcome else {
            panic!("expected bounded support facts");
        };
        assert_eq!(data.records.len(), 1);
        assert_eq!(data.analyzed_files.len(), 3);
        assert_eq!(data.contributors.len(), 1);
        assert_eq!(
            data.coverage,
            ProviderCoverage::new(
                3,
                3,
                data.analyzed_files.iter().map(|file| file.bytes).sum(),
                1,
            )
        );
        assert_eq!(diagnostic.code, "support_state_evidence_bound");
    }

    #[test]
    fn truncated_inventory_cannot_prove_that_objects_are_not_on_support() {
        let mut inventory = inventory(vec![descriptor(
            "Documents/Purchase.xml",
            "Document",
            "Purchase",
            LOCKED_UUID,
        )]);
        inventory.coverage.files_seen += 1;

        let outcome = SupportStateProvider.support(&query(100), &inventory);

        let ProviderOutcome::Bounded { data, diagnostic } = outcome else {
            panic!("truncated inventory must keep support discovery bounded");
        };
        assert!(data.records.is_empty());
        assert_eq!(diagnostic.code, "support_state_inventory_bounded");
    }

    fn state_for(
        records: &[crate::domain::discovery::SupportFact],
        artifact: &str,
    ) -> Option<SupportStateKind> {
        let artifact = ArtifactId::parse(artifact).expect("valid test artifact");
        records
            .iter()
            .find(|fact| fact.artifact == artifact)
            .map(|fact| fact.state)
    }

    fn descriptor(path: &str, kind: &str, name: &str, uuid: &str) -> SourceFile {
        let bytes = format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\">\n  <{kind} uuid=\"{uuid}\">\n    <Properties><Name>{name}</Name></Properties>\n  </{kind}>\n</MetaDataObject>"
        );
        source_file(path, bytes.as_bytes())
    }

    fn support_bytes(global_flag: u8, rules: &[(&str, u8)]) -> Vec<u8> {
        let rules = rules
            .iter()
            .map(|(uuid, flag)| format!("{flag},0,{uuid}"))
            .collect::<Vec<_>>()
            .join(",");
        format!("{{6,{global_flag},1,\"1.0\",\"Vendor\",\"Configuration\",{rules}}}").into_bytes()
    }

    fn query(max_evidence: u16) -> DiscoveryQuery<'static> {
        DiscoveryQuery::new(
            "support",
            &[],
            &[],
            &[],
            DiscoveryQueryLimits {
                max_files: 100,
                max_bytes: 1_000_000,
                max_evidence,
                max_candidates: 100,
                max_graph_depth: 12,
            },
        )
    }

    fn source_file(path: &str, bytes: &[u8]) -> SourceFile {
        SourceFile {
            relative_path: PortableRelativePath::parse_str(path).expect("portable test path"),
            bytes: bytes.to_vec(),
            raw_hash: ContentHash::sha256(bytes),
        }
    }

    fn inventory(files: Vec<SourceFile>) -> SourceInventory {
        let bytes = files.iter().map(|file| file.bytes.len() as u64).sum();
        let count = files.len() as u32;
        SourceInventory {
            files,
            coverage: ProviderCoverage::new(count, count, bytes, count),
        }
    }
}
