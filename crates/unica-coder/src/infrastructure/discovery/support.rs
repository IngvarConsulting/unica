use crate::application::discovery::ports::SupportStatePort;
use crate::domain::discovery::{
    DiscoveryQuery, EvidenceLocation, FactBatch, ProviderDiagnostic, ProviderOutcome, SourceFile,
    SourceInventory, SupportFact, SupportStateKind,
};
use crate::infrastructure::discovery::metadata::{
    analyzed_file_map, build_batch, contributors_for_records, inventory_is_bounded,
    parse_inventory_catalog, MetadataNode,
};
use crate::infrastructure::native_operations::common::{
    parse_support_state_bytes, ParsedSupportState, SupportObjectRule,
};

pub(crate) struct SupportStateProvider;

impl SupportStatePort for SupportStateProvider {
    fn support(
        &self,
        query: &DiscoveryQuery<'_>,
        files: &SourceInventory,
    ) -> ProviderOutcome<FactBatch<SupportFact>> {
        if let Some(outcome) = crate::infrastructure::discovery::cancellation_outcome(query) {
            return outcome;
        }
        match collect_support_facts(query, files) {
            Ok(SupportCollection::Complete(batch)) => ProviderOutcome::Complete(batch),
            Ok(SupportCollection::Bounded { batch, diagnostic }) => ProviderOutcome::Bounded {
                data: batch,
                diagnostic,
            },
            Err(diagnostic)
                if crate::infrastructure::discovery::is_cancellation_diagnostic(&diagnostic) =>
            {
                ProviderOutcome::Failed(diagnostic)
            }
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
    crate::infrastructure::discovery::check_cancellation(query)?;
    let catalog = parse_inventory_catalog(query, inventory)?;
    let inventory_bounded = inventory_is_bounded(inventory);
    let mut analyzed_files = analyzed_file_map(&catalog);
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

    let mut records = Vec::new();
    if support_file.is_some() || !inventory_bounded {
        for node in catalog
            .nodes()
            .into_iter()
            .filter(|node| node.object_uuid.is_some())
        {
            crate::infrastructure::discovery::check_cancellation(query)?;
            records.push(support_fact(node, support_file, parsed.as_ref())?);
        }
    }
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
    node: &MetadataNode,
    support_file: Option<&SourceFile>,
    parsed: Option<&ParsedSupportState>,
) -> Result<SupportFact, ProviderDiagnostic> {
    let (state, location) = match (support_file, parsed) {
        (None, None) => (
            SupportStateKind::NotOnSupport,
            node.primary_location().cloned().ok_or_else(|| {
                ProviderDiagnostic::material(
                    "support_metadata_location_missing",
                    "metadata artifact has no evidence location",
                )
            })?,
        ),
        (Some(file), Some(state)) => {
            let object_uuid = node.object_uuid.as_deref();
            let object_rule = object_uuid.and_then(|uuid| state.object_rule(uuid));
            let (support_state, line) = if state.removed() {
                (SupportStateKind::Removed, state.removed_line())
            } else if !state.global_editing_enabled() {
                (SupportStateKind::Locked, state.global_flag_line())
            } else {
                match object_rule {
                    Some(rule) => {
                        let line = object_uuid
                            .and_then(|uuid| state.object_rule_line(uuid))
                            .ok_or_else(|| {
                                ProviderDiagnostic::material(
                                    "support_state_line_missing",
                                    "parsed object support rule has no evidence line",
                                )
                            })?;
                        let support_state = match rule {
                            SupportObjectRule::Locked => SupportStateKind::Locked,
                            SupportObjectRule::Editable => SupportStateKind::Editable,
                            SupportObjectRule::OffSupport => SupportStateKind::NotOnSupport,
                        };
                        (support_state, line)
                    }
                    None => (SupportStateKind::NotOnSupport, state.header_line()),
                }
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
        artifact: node.artifact.clone(),
        artifact_kind: node.artifact_kind,
        state,
        location,
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
    fn cancelled_query_stops_support_before_parsing_records() {
        let cancellation = crate::domain::cancellation::CancellationToken::new();
        cancellation.cancel();
        let query = query(100).with_cancellation(&cancellation);

        let outcome = SupportStateProvider.support(&query, &SourceInventory::empty());

        let ProviderOutcome::Failed(diagnostic) = outcome else {
            panic!("cancelled support must be a failed provider outcome");
        };
        assert_eq!(diagnostic.code, "discovery_cancelled");
    }

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
    fn support_evidence_points_to_the_decisive_global_object_or_header_field() {
        let implicit_uuid = "40000000-0000-0000-0000-000000000003";
        let global_lock = multiline_support_bytes(1, LOCKED_UUID, 2);
        let global_outcome = SupportStateProvider.support(
            &query(100),
            &inventory(vec![
                descriptor("Documents/Locked.xml", "Document", "Locked", LOCKED_UUID),
                source_file("Ext/ParentConfigurations.bin", &global_lock),
            ]),
        );
        let ProviderOutcome::Complete(global_batch) = global_outcome else {
            panic!("expected global-lock support facts");
        };
        assert_eq!(
            location_line_for(&global_batch.records, "Document.Locked"),
            Some(3),
            "global lock is decided by the global flag"
        );

        let object_editable = multiline_support_bytes(0, EDITABLE_UUID, 1);
        let object_outcome = SupportStateProvider.support(
            &query(100),
            &inventory(vec![
                descriptor(
                    "Documents/Editable.xml",
                    "Document",
                    "Editable",
                    EDITABLE_UUID,
                ),
                descriptor(
                    "Documents/Implicit.xml",
                    "Document",
                    "Implicit",
                    implicit_uuid,
                ),
                source_file("Ext/ParentConfigurations.bin", &object_editable),
            ]),
        );
        let ProviderOutcome::Complete(object_batch) = object_outcome else {
            panic!("expected object-specific support facts");
        };
        assert_eq!(
            location_line_for(&object_batch.records, "Document.Editable"),
            Some(12),
            "object state is decided by its rule flag, not its UUID"
        );
        assert_eq!(
            location_line_for(&object_batch.records, "Document.Implicit"),
            Some(2),
            "an absent object rule falls back to the parsed format header"
        );
    }

    #[test]
    fn removed_support_evidence_points_to_the_removed_marker() {
        let descriptor = descriptor(
            "Documents/Purchase.xml",
            "Document",
            "Purchase",
            LOCKED_UUID,
        );
        let serialized = inventory(vec![
            descriptor.clone(),
            source_file("Ext/ParentConfigurations.bin", b"{\n6,\n0,\n0\n}"),
        ]);
        let legacy = inventory(vec![
            descriptor,
            source_file("Ext/ParentConfigurations.bin", b"removed"),
        ]);

        let ProviderOutcome::Complete(serialized) =
            SupportStateProvider.support(&query(100), &serialized)
        else {
            panic!("expected serialized removed support facts");
        };
        let ProviderOutcome::Complete(legacy) = SupportStateProvider.support(&query(100), &legacy)
        else {
            panic!("expected legacy removed support facts");
        };
        assert_eq!(
            location_line_for(&serialized.records, "Document.Purchase"),
            Some(4)
        );
        assert_eq!(
            location_line_for(&legacy.records, "Document.Purchase"),
            Some(1)
        );
    }

    #[test]
    fn only_explicit_short_removed_markers_are_valid_discovery_inputs() {
        for marker in [b"".as_slice(), b"removed".as_slice(), b"{6,0,0}".as_slice()] {
            let outcome = SupportStateProvider.support(
                &query(100),
                &inventory(vec![
                    descriptor(
                        "Documents/Purchase.xml",
                        "Document",
                        "Purchase",
                        LOCKED_UUID,
                    ),
                    source_file("Ext/ParentConfigurations.bin", marker),
                ]),
            );
            let ProviderOutcome::Complete(batch) = outcome else {
                panic!("explicit removed marker must be complete: {marker:?}");
            };
            assert_eq!(
                state_for(&batch.records, "Document.Purchase"),
                Some(SupportStateKind::Removed)
            );
        }

        for malformed in [
            b"garbage".as_slice(),
            b"\xff".as_slice(),
            b"0".as_slice(),
            b"removed\n".as_slice(),
            b"{6,0".as_slice(),
        ] {
            let outcome = SupportStateProvider.support(
                &query(100),
                &inventory(vec![
                    descriptor(
                        "Documents/Purchase.xml",
                        "Document",
                        "Purchase",
                        LOCKED_UUID,
                    ),
                    source_file("Ext/ParentConfigurations.bin", malformed),
                ]),
            );
            assert!(
                matches!(outcome, ProviderOutcome::ContractViolation(_)),
                "ambiguous short payload must invalidate discovery: {malformed:?}"
            );
        }
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

    #[test]
    fn support_is_projected_to_recursive_and_subordinate_uuid_artifacts() {
        let root_uuid = "60000000-0000-0000-0000-000000000001";
        let section_uuid = "60000000-0000-0000-0000-000000000002";
        let form_uuid = "60000000-0000-0000-0000-000000000003";
        let parent = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Document uuid="{root_uuid}">
    <Properties><Name>Purchase</Name></Properties>
    <ChildObjects>
      <TabularSection uuid="{section_uuid}">
        <Properties><Name>Серии</Name></Properties>
      </TabularSection>
      <Form>Main</Form>
    </ChildObjects>
  </Document>
</MetaDataObject>"#
        );
        let form = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Form uuid="{form_uuid}"><Properties><Name>Main</Name></Properties></Form>
</MetaDataObject>"#
        );
        let support = support_bytes(0, &[(root_uuid, 0), (section_uuid, 1), (form_uuid, 2)]);
        let inventory = inventory(vec![
            source_file("Documents/Purchase.xml", parent.as_bytes()),
            source_file("Documents/Purchase/Forms/Main.xml", form.as_bytes()),
            source_file("Ext/ParentConfigurations.bin", &support),
        ]);

        let outcome = SupportStateProvider.support(&query(100), &inventory);

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("recursive support catalog should be complete");
        };
        assert_eq!(
            state_for(&batch.records, "Document.Purchase"),
            Some(SupportStateKind::Locked)
        );
        assert_eq!(
            state_for(&batch.records, "Document.Purchase.TabularSection.Серии"),
            Some(SupportStateKind::Editable)
        );
        assert_eq!(
            state_for(&batch.records, "Document.Purchase.Form.Main"),
            Some(SupportStateKind::NotOnSupport)
        );
        assert_eq!(
            batch
                .records
                .iter()
                .find(|fact| {
                    fact.artifact
                        == ArtifactId::parse("Document.Purchase.TabularSection.Серии")
                            .expect("valid section artifact")
                })
                .map(|fact| fact.artifact_kind),
            Some(crate::domain::discovery::ArtifactKind::TabularSection)
        );
        assert_eq!(
            batch
                .records
                .iter()
                .find(|fact| {
                    fact.artifact
                        == ArtifactId::parse("Document.Purchase.Form.Main")
                            .expect("valid form artifact")
                })
                .map(|fact| fact.artifact_kind),
            Some(crate::domain::discovery::ArtifactKind::Form)
        );
    }

    #[test]
    fn tracked_configuration_catalog_support_uses_one_canonical_claim_per_object() {
        let outcome =
            SupportStateProvider.support(&query(100), &tracked_meta_compile_on_support_inventory());

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("full tracked meta-compile inventory must be complete");
        };
        assert_eq!(
            state_for(&batch.records, "Configuration.ТестКонфиг"),
            Some(SupportStateKind::Locked)
        );
        assert_eq!(
            state_for(&batch.records, "Catalog.Locked"),
            Some(SupportStateKind::Locked)
        );
        assert_eq!(
            state_for(&batch.records, "Catalog.Removed"),
            Some(SupportStateKind::NotOnSupport)
        );
        assert_eq!(batch.records.len(), 3);
        assert_eq!(
            batch
                .records
                .iter()
                .filter(|fact| fact.artifact == ArtifactId::parse("Catalog.Locked").unwrap())
                .count(),
            1
        );
        assert!(batch.records.iter().all(|fact| {
            !fact
                .artifact
                .as_str()
                .starts_with("Configuration.ТестКонфиг.Catalog.")
        }));
    }

    #[test]
    fn tracked_template_support_uuid_maps_only_to_its_parent_canonical_identity() {
        let mut inventory = inventory(vec![
            source_file(
                "Reports/ParityReport.xml",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/template-remove/ParityReport.xml"
                )),
            ),
            source_file(
                "Reports/ParityReport/Templates/MainSchema.xml",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/template-remove/",
                    "ParityReport/Templates/MainSchema.xml"
                )),
            ),
            source_file(
                "Ext/ParentConfigurations.bin",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/",
                    "meta-compile/fixtures/on-support/Ext/ParentConfigurations.bin"
                )),
            ),
        ]);
        inventory.coverage.files_seen += 1;

        let outcome = SupportStateProvider.support(&query(100), &inventory);

        let ProviderOutcome::Bounded { data, diagnostic } = outcome else {
            panic!("unresolved tracked sibling must keep support discovery bounded");
        };
        assert_eq!(diagnostic.code, "support_state_inventory_bounded");
        assert_eq!(
            state_for(&data.records, "Report.ParityReport.Template.MainSchema"),
            Some(SupportStateKind::Locked)
        );
        assert_eq!(state_for(&data.records, "Template.MainSchema"), None);
    }

    #[test]
    fn unresolved_declaration_never_yields_complete_support_absence() {
        let parent = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Report uuid="61000000-0000-0000-0000-000000000001">
    <Properties><Name>Sales</Name></Properties>
    <ChildObjects><Template>Main</Template></ChildObjects>
  </Report>
</MetaDataObject>"#;
        let complete = inventory(vec![source_file("Reports/Sales.xml", parent.as_bytes())]);
        let mut bounded = complete.clone();
        bounded.coverage.files_seen += 1;

        let complete_outcome = SupportStateProvider.support(&query(100), &complete);
        let bounded_outcome = SupportStateProvider.support(&query(100), &bounded);

        assert!(matches!(
            complete_outcome,
            ProviderOutcome::ContractViolation(_)
        ));
        let ProviderOutcome::Bounded { data, diagnostic } = bounded_outcome else {
            panic!("bounded inventory must not invent complete support absence");
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

    fn location_line_for(
        records: &[crate::domain::discovery::SupportFact],
        artifact: &str,
    ) -> Option<u32> {
        let artifact = ArtifactId::parse(artifact).expect("valid test artifact");
        records
            .iter()
            .find(|fact| fact.artifact == artifact)
            .and_then(|fact| fact.location.line)
    }

    fn descriptor(path: &str, kind: &str, name: &str, uuid: &str) -> SourceFile {
        let bytes = format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\">\n  <{kind} uuid=\"{uuid}\">\n    <Properties><Name>{name}</Name></Properties>\n  </{kind}>\n</MetaDataObject>"
        );
        source_file(path, bytes.as_bytes())
    }

    fn support_bytes(global_flag: u8, rules: &[(&str, u8)]) -> Vec<u8> {
        let object_count = rules.len();
        let rules = rules
            .iter()
            .map(|(uuid, flag)| format!("{flag},0,{uuid},{uuid}"))
            .collect::<Vec<_>>()
            .join(",");
        let separator = if rules.is_empty() { "" } else { "," };
        format!(
            "{{6,{global_flag},1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"Configuration\",{}{separator}{rules}}}",
            object_count
        )
        .into_bytes()
    }

    fn multiline_support_bytes(global_flag: u8, uuid: &str, rule: u8) -> Vec<u8> {
        format!(
            "{{\n6,\n{global_flag},\n1,\ndddddddd-dddd-dddd-dddd-dddddddddddd,\n0,\neeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\n\"1.0\",\n\"Vendor\",\n\"Configuration\",\n1,\n{rule},\n0,\n{uuid},\n{uuid}\n}}"
        )
        .into_bytes()
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
            bytes: bytes.to_vec().into(),
            raw_hash: ContentHash::sha256(bytes),
        }
    }

    fn tracked_meta_compile_on_support_inventory() -> SourceInventory {
        inventory(vec![
            source_file(
                "Configuration.xml",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/",
                    "meta-compile/fixtures/on-support/Configuration.xml"
                )),
            ),
            source_file(
                "Catalogs/Locked.xml",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/",
                    "meta-compile/fixtures/on-support/Catalogs/Locked.xml"
                )),
            ),
            source_file(
                "Catalogs/Removed.xml",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/",
                    "meta-compile/fixtures/on-support/Catalogs/Removed.xml"
                )),
            ),
            source_file(
                "Ext/ParentConfigurations.bin",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/",
                    "meta-compile/fixtures/on-support/Ext/ParentConfigurations.bin"
                )),
            ),
        ])
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
