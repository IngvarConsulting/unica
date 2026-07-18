pub(crate) mod contract;
pub(crate) mod determinism;
pub(crate) mod evidence_graph;
pub(crate) mod model;
pub(crate) mod ports;
pub(crate) mod proposal_validator;
pub(crate) mod use_case;

#[cfg(test)]
mod tests {
    use super::contract::*;
    use super::determinism::*;
    use super::model::*;
    use crate::domain::discovery_registry::{metadata_kind, METADATA_KIND_TAGS, MODULE_KIND_TAGS};
    use crate::domain::project_sources::SourceFormat;
    use serde_json::json;

    fn request(value: serde_json::Value) -> Result<DiscoverRequest, serde_json::Error> {
        serde_json::from_value(value)
    }

    fn minimal_explore() -> serde_json::Value {
        json!({"mode": "explore", "task": "Find hook", "concepts": ["write"]})
    }

    fn minimal_validate() -> serde_json::Value {
        json!({
            "mode": "validate",
            "task": "Patch write",
            "concepts": ["write"],
            "proposals": [{
                "id": "write-hook",
                "target": {"kind": "method", "ref": "CommonModule.Flow.Run"},
                "intent": "Run before write"
            }]
        })
    }

    #[test]
    fn strict_request_mode_and_cardinality_contract() {
        assert!(request(minimal_explore()).is_ok());
        assert!(request(minimal_validate()).is_ok());

        for bad in [
            json!({"mode":"explore","task":"x","concepts":["x"],"extra":1}),
            json!({"mode":"explore","task":"x","concepts":["x"],"proposals":[]}),
            json!({"mode":"explore","task":"x","concepts":["x"],"proposals":null}),
            json!({"mode":"validate","task":"x","concepts":["x"]}),
            json!({"mode":"validate","task":"x","concepts":["x"],"proposals":[]}),
            json!({"mode":"explore","task":" ","concepts":["x"]}),
            json!({"mode":"explore","task":"x","concepts":[]}),
            json!({"mode":"explore","task":"x","concepts":["x","x"]}),
            json!({"mode":"explore","task":"x","concepts":["x"],"searchTerms":["a","a"]}),
            json!({"mode":"explore","task":"x","concepts":["x"],"knownArtifacts":[
                {"kind":"method","ref":"CommonModule.X.Run"},
                {"kind":"method","ref":"CommonModule.x.run"}
            ]}),
        ] {
            assert!(request(bad).is_err(), "accepted invalid request");
        }
        assert!(request(json!({
            "mode":"explore","task":" x ","concepts":["x","X"],"searchTerms":["a","A"]
        }))
        .is_ok());
        let preserved = request(json!({
            "mode":"explore","task":" x ","concepts":[" x "]
        }))
        .unwrap();
        assert_eq!(preserved.task, " x ");
        assert_eq!(preserved.concepts, [" x "]);

        let task_8192 = "x".repeat(8192);
        assert!(request(json!({"mode":"explore","task":task_8192,"concepts":["x"]})).is_ok());
        let task_8193 = "x".repeat(8193);
        assert!(request(json!({"mode":"explore","task":task_8193,"concepts":["x"]})).is_err());

        let concepts_64: Vec<_> = (0..64).map(|i| format!("c{i}")).collect();
        assert!(request(json!({"mode":"explore","task":"x","concepts":concepts_64})).is_ok());
        let concepts_65: Vec<_> = (0..65).map(|i| format!("c{i}")).collect();
        assert!(request(json!({"mode":"explore","task":"x","concepts":concepts_65})).is_err());

        let searches_128: Vec<_> = (0..128).map(|i| format!("s{i}")).collect();
        assert!(request(
            json!({"mode":"explore","task":"x","concepts":["x"],"searchTerms":searches_128})
        )
        .is_ok());
        let searches_129: Vec<_> = (0..129).map(|i| format!("s{i}")).collect();
        assert!(request(
            json!({"mode":"explore","task":"x","concepts":["x"],"searchTerms":searches_129})
        )
        .is_err());

        let artifacts_128: Vec<_> = (0..128)
            .map(|i| json!({"kind":"module","ref":format!("CommonModule.M{i}")}))
            .collect();
        assert!(request(
            json!({"mode":"explore","task":"x","concepts":["x"],"knownArtifacts":artifacts_128})
        )
        .is_ok());
        let artifacts_129: Vec<_> = (0..129)
            .map(|i| json!({"kind":"module","ref":format!("CommonModule.M{i}")}))
            .collect();
        assert!(request(
            json!({"mode":"explore","task":"x","concepts":["x"],"knownArtifacts":artifacts_129})
        )
        .is_err());
    }

    #[test]
    fn strict_string_and_proposal_boundaries() {
        let text_256 = "я".repeat(128);
        assert_eq!(text_256.len(), 256);
        assert!(request(json!({"mode":"explore","task":"x","concepts":[text_256]})).is_ok());
        let text_258 = "я".repeat(129);
        assert!(request(json!({"mode":"explore","task":"x","concepts":[text_258]})).is_err());

        let intent_2048 = "x".repeat(2048);
        let mut valid = minimal_validate();
        valid["proposals"][0]["intent"] = json!(intent_2048);
        assert!(request(valid).is_ok());
        let mut too_long = minimal_validate();
        too_long["proposals"][0]["intent"] = json!("x".repeat(2049));
        assert!(request(too_long).is_err());

        for id in ["a", "A.0_b-c", &"x".repeat(64)] {
            let mut value = minimal_validate();
            value["proposals"][0]["id"] = json!(id);
            assert!(request(value).is_ok(), "rejected id {id}");
        }
        for id in ["", "-bad", ".bad", "bad space", &"x".repeat(65)] {
            let mut value = minimal_validate();
            value["proposals"][0]["id"] = json!(id);
            assert!(request(value).is_err(), "accepted id {id}");
        }

        let mut duplicate = minimal_validate();
        let first = duplicate["proposals"][0].clone();
        duplicate["proposals"].as_array_mut().unwrap().push(first);
        assert!(request(duplicate).is_err());

        let proposals_32: Vec<_> = (0..32)
            .map(|i| {
                json!({
                    "id": format!("p{i}"),
                    "target": {"kind":"method","ref":"CommonModule.Flow.Run"},
                    "intent":"x"
                })
            })
            .collect();
        assert!(request(
            json!({"mode":"validate","task":"x","concepts":["x"],"proposals":proposals_32})
        )
        .is_ok());
        let proposals_33: Vec<_> = (0..33)
            .map(|i| {
                json!({
                    "id": format!("p{i}"),
                    "target": {"kind":"method","ref":"CommonModule.Flow.Run"},
                    "intent":"x"
                })
            })
            .collect();
        assert!(request(
            json!({"mode":"validate","task":"x","concepts":["x"],"proposals":proposals_33})
        )
        .is_err());

        let mut source_boundary = minimal_explore();
        source_boundary["sourceSet"] = json!("x".repeat(1024));
        assert!(request(source_boundary).is_ok());
        let mut source_too_long = minimal_explore();
        source_too_long["sourceSet"] = json!("x".repeat(1025));
        assert!(request(source_too_long).is_err());
    }

    #[test]
    fn limits_have_exact_defaults_ranges_and_strict_shape() {
        let parsed = request(minimal_explore()).unwrap();
        assert_eq!(parsed.limits, DiscoverLimits::default());
        assert_eq!(parsed.limits.max_candidates, 20);
        assert_eq!(parsed.limits.max_graph_depth, 4);
        assert_eq!(parsed.limits.max_evidence, 200);

        for limits in [
            json!({"maxCandidates":1,"maxGraphDepth":1,"maxEvidence":1}),
            json!({"maxCandidates":100,"maxGraphDepth":12,"maxEvidence":2000}),
            json!({"maxCandidates":20}),
        ] {
            let mut value = minimal_explore();
            value["limits"] = limits;
            assert!(request(value).is_ok());
        }
        for limits in [
            json!({"maxCandidates":0}),
            json!({"maxCandidates":101}),
            json!({"maxGraphDepth":0}),
            json!({"maxGraphDepth":13}),
            json!({"maxEvidence":0}),
            json!({"maxEvidence":2001}),
            json!({"maxCandidates":null}),
            json!({"maxGraphDepth":null}),
            json!({"maxEvidence":null}),
            json!({"maxCandidates":20,"score":1}),
        ] {
            let mut value = minimal_explore();
            value["limits"] = limits;
            assert!(request(value).is_err());
        }
    }

    #[test]
    fn search_terms_have_exact_blank_and_utf8_byte_boundaries() {
        for term in ["", " ", "\n\t"] {
            assert!(request(json!({
                "mode":"explore","task":"x","concepts":["x"],"searchTerms":[term]
            }))
            .is_err());
        }

        let boundary = "я".repeat(128);
        assert_eq!(boundary.len(), 256);
        assert!(request(json!({
            "mode":"explore","task":"x","concepts":["x"],"searchTerms":[boundary]
        }))
        .is_ok());
        let over = format!("{}x", "я".repeat(128));
        assert_eq!(over.len(), 257);
        assert!(request(json!({
            "mode":"explore","task":"x","concepts":["x"],"searchTerms":[over]
        }))
        .is_err());
    }

    #[test]
    fn nested_inputs_reject_unknown_fields_and_scalar_confidence() {
        let mut target_extra = minimal_validate();
        target_extra["proposals"][0]["target"]["confidence"] = json!(1);
        assert!(request(target_extra)
            .unwrap_err()
            .to_string()
            .contains("unknown field"));

        let mut proposal_extra = minimal_validate();
        proposal_extra["proposals"][0]["score"] = json!(1);
        assert!(request(proposal_extra)
            .unwrap_err()
            .to_string()
            .contains("unknown field"));

        let mut artifact_extra = minimal_explore();
        artifact_extra["knownArtifacts"] =
            json!([{"kind":"module","ref":"CommonModule.X","path":"x"}]);
        assert!(request(artifact_extra)
            .unwrap_err()
            .to_string()
            .contains("unknown field"));

        let mut null_intent = minimal_validate();
        null_intent["proposals"][0]["mutationIntent"] = serde_json::Value::Null;
        assert!(request(null_intent).is_err());
        let mut null_source = minimal_explore();
        null_source["sourceSet"] = serde_json::Value::Null;
        assert!(request(null_source).is_err());
    }

    #[test]
    fn cfe_patch_method_intent_is_the_only_strict_tagged_variant() {
        let mut value = minimal_validate();
        value["proposals"][0]["mutationIntent"] = json!({
            "tool":"unica.cfe.patch_method",
            "destinationSourceSet":"extension",
            "arguments": {
                "ExtensionPath":"src-cfe",
                "ModulePath":"Documents.X.ObjectModule",
                "MethodName":"BeforeWrite",
                "InterceptorType":"Before"
            }
        });
        let parsed = request(value.clone()).unwrap();
        let MutationIntent::CfePatchMethod { arguments, .. } =
            parsed.proposals[0].mutation_intent.as_ref().unwrap();
        assert_eq!(arguments.context, ExecutionContext::Server);
        assert!(!arguments.is_function);

        value["proposals"][0]["mutationIntent"]["arguments"]["Context"] = json!("НаКлиенте");
        value["proposals"][0]["mutationIntent"]["arguments"]["IsFunction"] = json!(true);
        let parsed = request(value.clone()).unwrap();
        let MutationIntent::CfePatchMethod { arguments, .. } =
            parsed.proposals[0].mutation_intent.as_ref().unwrap();
        assert_eq!(arguments.context, ExecutionContext::Client);
        assert!(arguments.is_function);

        for mutation in [
            json!({"tool":"unica.cf.edit","destinationSourceSet":"x","arguments":{}}),
            json!({"tool":"unica.cfe.patch_method","destinationSourceSet":"x","arguments":{
                "ExtensionPath":"x","ModulePath":"x","MethodName":"x","InterceptorType":"Before","force":true
            }}),
            json!({"tool":"unica.cfe.patch_method","destinationSourceSet":"x","arguments":{
                "ExtensionPath":"x","ModulePath":"x","MethodName":"x","InterceptorType":"Around"
            }}),
            json!({"tool":"unica.cfe.patch_method","destinationSourceSet":" ","arguments":{
                "ExtensionPath":"x","ModulePath":"x","MethodName":"x","InterceptorType":"Before"
            }}),
        ] {
            let mut bad = minimal_validate();
            bad["proposals"][0]["mutationIntent"] = mutation;
            assert!(request(bad).is_err());
        }

        let mut resolver_boundary = minimal_validate();
        resolver_boundary["proposals"][0]["mutationIntent"] = json!({
            "tool":"unica.cfe.patch_method",
            "destinationSourceSet":"x".repeat(1024),
            "arguments": {
                "ExtensionPath":"x".repeat(1024),
                "ModulePath":"x".repeat(1024),
                "MethodName":"я".repeat(128),
                "InterceptorType":"Before"
            }
        });
        assert!(request(resolver_boundary).is_ok());
        let mut resolver_too_long = minimal_validate();
        resolver_too_long["proposals"][0]["mutationIntent"] = json!({
            "tool":"unica.cfe.patch_method",
            "destinationSourceSet":"x",
            "arguments": {
                "ExtensionPath":"x".repeat(1025),
                "ModulePath":"x",
                "MethodName":"x",
                "InterceptorType":"Before"
            }
        });
        assert!(request(resolver_too_long).is_err());
        let mut identifier_too_long = minimal_validate();
        identifier_too_long["proposals"][0]["mutationIntent"] = json!({
            "tool":"unica.cfe.patch_method",
            "destinationSourceSet":"x",
            "arguments": {
                "ExtensionPath":"x",
                "ModulePath":"x",
                "MethodName":"я".repeat(129),
                "InterceptorType":"Before"
            }
        });
        assert!(request(identifier_too_long).is_err());
    }

    #[test]
    fn every_resolver_string_enforces_blank_and_byte_boundaries() {
        fn mutation() -> serde_json::Value {
            json!({
                "tool":"unica.cfe.patch_method",
                "destinationSourceSet":"extension",
                "arguments": {
                    "ExtensionPath":"src-cfe",
                    "ModulePath":"Documents.X.ObjectModule",
                    "MethodName":"BeforeWrite",
                    "InterceptorType":"Before"
                }
            })
        }

        for field in [
            "destinationSourceSet",
            "ExtensionPath",
            "ModulePath",
            "MethodName",
        ] {
            let mut value = minimal_validate();
            value["proposals"][0]["mutationIntent"] = mutation();
            if field == "destinationSourceSet" {
                value["proposals"][0]["mutationIntent"][field] = json!(" ");
            } else {
                value["proposals"][0]["mutationIntent"]["arguments"][field] = json!(" ");
            }
            assert!(request(value).is_err(), "accepted blank {field}");
        }

        for field in ["destinationSourceSet", "ExtensionPath", "ModulePath"] {
            let mut boundary = minimal_validate();
            boundary["proposals"][0]["mutationIntent"] = mutation();
            if field == "destinationSourceSet" {
                boundary["proposals"][0]["mutationIntent"][field] = json!("x".repeat(1024));
            } else {
                boundary["proposals"][0]["mutationIntent"]["arguments"][field] =
                    json!("x".repeat(1024));
            }
            assert!(request(boundary).is_ok(), "rejected 1024-byte {field}");

            let mut over = minimal_validate();
            over["proposals"][0]["mutationIntent"] = mutation();
            if field == "destinationSourceSet" {
                over["proposals"][0]["mutationIntent"][field] = json!("x".repeat(1025));
            } else {
                over["proposals"][0]["mutationIntent"]["arguments"][field] =
                    json!("x".repeat(1025));
            }
            assert!(request(over).is_err(), "accepted 1025-byte {field}");
        }

        for length in [1024, 1025] {
            let mut value = minimal_validate();
            value["proposals"][0]["mutationIntent"] = mutation();
            value["proposals"][0]["mutationIntent"]["arguments"]["MethodName"] =
                json!("x".repeat(length));
            assert!(request(value).is_err(), "accepted oversized MethodName");
        }
    }

    #[test]
    fn omitted_and_explicit_resolver_defaults_are_identical() {
        let mut omitted = minimal_validate();
        omitted["proposals"][0]["mutationIntent"] = json!({
            "tool":"unica.cfe.patch_method",
            "destinationSourceSet":"extension",
            "arguments": {
                "ExtensionPath":"src-cfe",
                "ModulePath":"Documents.X.ObjectModule",
                "MethodName":"BeforeWrite",
                "InterceptorType":"Before"
            }
        });
        let mut explicit = omitted.clone();
        explicit["proposals"][0]["mutationIntent"]["arguments"]["Context"] = json!("НаСервере");
        explicit["proposals"][0]["mutationIntent"]["arguments"]["IsFunction"] = json!(false);
        assert_eq!(request(omitted).unwrap(), request(explicit).unwrap());
    }

    #[test]
    fn all_fifteen_canonical_artifact_shapes_round_trip() {
        let cases = [
            (ArtifactKind::MetadataObject, "Document.Sale"),
            (
                ArtifactKind::MetadataAttribute,
                "Document.Sale.Attribute.Number",
            ),
            (
                ArtifactKind::TabularSection,
                "Document.Sale.TabularSection.Goods",
            ),
            (
                ArtifactKind::TabularSectionAttribute,
                "Document.Sale.TabularSection.Goods.Attribute.Item",
            ),
            (ArtifactKind::Module, "CommonModule.Flow"),
            (ArtifactKind::Method, "CommonModule.Flow.Run"),
            (ArtifactKind::Form, "Document.Sale.Form.Main"),
            (
                ArtifactKind::FormCommand,
                "Document.Sale.Form.Main.Command.Post",
            ),
            (ArtifactKind::CommonCommand, "CommonCommand.Refresh"),
            (
                ArtifactKind::EventSubscription,
                "EventSubscription.BeforeWrite",
            ),
            (ArtifactKind::ScheduledJob, "ScheduledJob.Refresh"),
            (
                ArtifactKind::HttpRoute,
                "HTTPService.Api.URLTemplate.Items.Method.Get",
            ),
            (ArtifactKind::ExchangePlan, "ExchangePlan.Sync"),
            (ArtifactKind::Report, "Report.Sales"),
            (ArtifactKind::DataProcessor, "DataProcessor.Import"),
        ];
        assert_eq!(cases.len(), ArtifactKind::ALL.len());
        for (kind, canonical_ref) in cases {
            let artifact = ArtifactRef::parse(kind, canonical_ref).unwrap();
            let encoded = serde_json::to_value(&artifact).unwrap();
            assert_eq!(
                serde_json::from_value::<ArtifactRef>(encoded).unwrap(),
                artifact
            );
        }

        assert!(ArtifactRef::parse(ArtifactKind::Module, "Document.Sale.ObjectModule").is_ok());
        assert!(ArtifactRef::parse(ArtifactKind::Module, "HTTPService.Api.Module").is_ok());
        assert!(
            ArtifactRef::parse(ArtifactKind::Method, "HTTPService.Api.Module.GetHandler").is_ok()
        );
        assert!(
            ArtifactRef::parse(ArtifactKind::Module, "Document.Sale.Form.Main.FormModule").is_ok()
        );
        assert!(ArtifactRef::parse(
            ArtifactKind::Method,
            "Document.Sale.ObjectModule.BeforeWrite"
        )
        .is_ok());
        assert!(ArtifactRef::parse(
            ArtifactKind::Method,
            "Document.Sale.Form.Main.FormModule.Open"
        )
        .is_ok());
    }

    #[test]
    fn artifact_parser_uses_the_single_versioned_domain_registry() {
        assert!(metadata_kind("Document").is_some());
        assert!(METADATA_KIND_TAGS.contains(&"IntegrationService"));
        assert_eq!(
            MODULE_KIND_TAGS,
            &[
                "Module",
                "ObjectModule",
                "ManagerModule",
                "RecordSetModule",
                "ValueManagerModule",
                "CommandModule",
            ]
        );
        for module_kind in MODULE_KIND_TAGS {
            let canonical_ref = format!("Document.X.{module_kind}");
            assert!(ArtifactRef::parse(ArtifactKind::Module, &canonical_ref).is_ok());
        }
    }

    #[test]
    fn canonical_artifacts_reject_cross_kind_unregistered_and_traversal_shapes() {
        for (kind, canonical_ref) in [
            (ArtifactKind::MetadataObject, "UnknownKind.X"),
            (ArtifactKind::MetadataObject, "Report.X"),
            (ArtifactKind::MetadataObject, "CommonModule.X"),
            (ArtifactKind::Module, "UnknownKind.X.ObjectModule"),
            (ArtifactKind::Module, "Document.X.UnknownModule"),
            (ArtifactKind::Module, "CommonModule.X.ObjectModule"),
            (ArtifactKind::Method, "Document.X.BeforeWrite"),
            (ArtifactKind::Form, "Document.X.Command.Run"),
            (ArtifactKind::CommonCommand, "Document.X"),
            (
                ArtifactKind::HttpRoute,
                "HTTPService.Api.UrlTemplate.X.Method.Get",
            ),
            (ArtifactKind::Report, "DataProcessor.X"),
            (ArtifactKind::Method, "CommonModule..Run"),
            (ArtifactKind::Method, "CommonModule...Run"),
            (ArtifactKind::Method, "CommonModule.X/Run"),
            (ArtifactKind::Method, "CommonModule.X\\Run"),
            (ArtifactKind::Method, ".CommonModule.X.Run"),
        ] {
            assert!(
                ArtifactRef::parse(kind, canonical_ref).is_err(),
                "accepted {kind:?} {canonical_ref}"
            );
        }
        let too_long_segment = format!("CommonModule.{}.Run", "x".repeat(129));
        assert!(ArtifactRef::parse(ArtifactKind::Method, &too_long_segment).is_err());
        let unicode_boundary = format!("CommonModule.{}.Run", "я".repeat(128));
        assert!(ArtifactRef::parse(ArtifactKind::Method, &unicode_boundary).is_ok());
        let unicode_too_long = format!("CommonModule.{}.Run", "я".repeat(129));
        assert!(ArtifactRef::parse(ArtifactKind::Method, &unicode_too_long).is_err());

        for root in [
            "CommonModule",
            "CommonCommand",
            "EventSubscription",
            "ScheduledJob",
            "ExchangePlan",
            "Report",
            "DataProcessor",
        ] {
            assert!(
                ArtifactRef::parse(ArtifactKind::MetadataObject, &format!("{root}.X")).is_err(),
                "accepted specialized metadata root {root}"
            );
        }

        let exact_1024 = format!(
            "Document.{}.Form.{}.FormModule.{}x",
            "界".repeat(128),
            "界".repeat(128),
            "界".repeat(76)
        );
        assert_eq!(exact_1024.len(), 1024);
        assert!(ArtifactRef::parse(ArtifactKind::Method, &exact_1024).is_ok());
        let over_1024 = format!("{exact_1024}x");
        assert_eq!(over_1024.len(), 1025);
        assert!(ArtifactRef::parse(ArtifactKind::Method, &over_1024).is_err());
    }

    #[test]
    fn artifact_identity_is_case_insensitive_but_serialization_preserves_spelling() {
        let upper = ArtifactRef::parse(ArtifactKind::Method, "CommonModule.Flow.Run").unwrap();
        let lower = ArtifactRef::parse(ArtifactKind::Method, "CommonModule.flow.run").unwrap();
        assert_eq!(upper, lower);
        assert_eq!(upper.cmp(&lower), std::cmp::Ordering::Equal);
        assert_eq!(
            serde_json::to_value(upper).unwrap()["ref"],
            "CommonModule.Flow.Run"
        );
    }

    #[test]
    fn check_wire_shape_is_exact_and_details_are_bounded() {
        let value = json!({
            "code":"call_graph",
            "provider":"CallGraphPort",
            "state":"unavailable",
            "outcome":"inconclusive",
            "coverage":"unknown",
            "severity":"blocking",
            "affects":["proposal:p"],
            "reasonCode":"index_building",
            "retryable":true,
            "details":["workspace index is building"],
            "evidenceIds":[]
        });
        let check: Check = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(check).unwrap(), value);

        let mut extra = value.clone();
        extra["confidence"] = json!(1);
        assert!(serde_json::from_value::<Check>(extra).is_err());
        let mut too_many = value.clone();
        too_many["details"] = json!((0..33).map(|i| format!("d{i}")).collect::<Vec<_>>());
        assert!(serde_json::from_value::<Check>(too_many).is_err());
        let mut too_long = value;
        too_long["details"] = json!(["x".repeat(513)]);
        assert!(serde_json::from_value::<Check>(too_long).is_err());
    }

    #[test]
    fn source_location_rejects_absolute_traversal_and_zero_coordinates() {
        for (path, line, column) in [
            ("/absolute/Module.bsl", Some(1), Some(1)),
            ("C:/absolute/Module.bsl", Some(1), Some(1)),
            ("C:drive-relative/Module.bsl", Some(1), Some(1)),
            ("//server/share/Module.bsl", Some(1), Some(1)),
            (r"\\server\share\Module.bsl", Some(1), Some(1)),
            (r"\\?\C:\outside\Module.bsl", Some(1), Some(1)),
            (r"\\.\PIPE\name", Some(1), Some(1)),
            ("Module.bsl:stream", Some(1), Some(1)),
            ("src/Module.bsl:stream", Some(1), Some(1)),
            ("../escape.bsl", Some(1), Some(1)),
            ("a/./b.bsl", Some(1), Some(1)),
            ("a\\b.bsl", Some(1), Some(1)),
            ("a.bsl", Some(0), Some(1)),
            ("a.bsl", Some(1), Some(0)),
        ] {
            assert!(SourceLocation::new(path, line, column).is_err());
        }
        assert!(SourceLocation::new("src/Module.bsl", Some(1), Some(1)).is_ok());
    }

    fn record(subject: &str, provider: &str, epoch: u64) -> EvidenceRecord {
        EvidenceRecord::from_fact(
            ProviderFact::DefinitionPresent {
                subject: ArtifactRef::parse(ArtifactKind::Method, subject).unwrap(),
                definition: DefinitionShape::new(false, true, vec![]).unwrap(),
            },
            Some(
                SourceLocation::new("CommonModules/Flow/Ext/Module.bsl", Some(2), Some(1)).unwrap(),
            ),
            EvidenceProvider::new(EvidencePort::Definition, provider, "1").unwrap(),
            Coverage::Complete,
            Freshness::new("main", &format!("sha256:{}", "1".repeat(64)), epoch).unwrap(),
        )
    }

    fn outcomes(order: bool) -> Vec<ProviderOutcomeSnapshot> {
        let a = ProviderOutcomeSnapshot::new(
            EvidencePort::Definition,
            "defs",
            "1",
            ProviderReadiness::Ready,
            Coverage::Complete,
            None,
            vec![format!("sha256:{}", "a".repeat(64))],
        )
        .unwrap();
        let b = ProviderOutcomeSnapshot::new(
            EvidencePort::CallGraph,
            "calls",
            "2",
            ProviderReadiness::Unavailable,
            Coverage::Unknown,
            Some("index_building".to_string()),
            vec![],
        )
        .unwrap();
        if order {
            vec![a, b]
        } else {
            vec![b, a]
        }
    }

    fn discovery_source(epoch: u64, fingerprint_digit: char) -> DiscoverySource {
        let fingerprint = format!("sha256:{}", fingerprint_digit.to_string().repeat(64));
        DiscoverySource {
            analysis_source_set: "main".to_string(),
            source_format: SourceFormat::PlatformXml,
            workspace_epoch: epoch,
            linked_source_snapshots: vec![LinkedSourceSnapshot {
                source_set: "main".to_string(),
                role: SourceSnapshotRole::Analysis,
                source_fingerprint: fingerprint.clone(),
            }],
            composite_source_fingerprint: fingerprint,
        }
    }

    #[test]
    fn stable_ids_ignore_permutation_and_diagnostic_epoch() {
        let mut first_value = minimal_explore();
        first_value["concepts"] = json!(["write", "goods"]);
        first_value["searchTerms"] = json!(["BeforeWrite", "Goods"]);
        let first = request(first_value).unwrap();
        let mut second_value = minimal_explore();
        second_value["concepts"] = json!(["goods", "write"]);
        second_value["searchTerms"] = json!(["Goods", "BeforeWrite"]);
        let second = request(second_value).unwrap();
        assert_eq!(
            analysis_id(
                &first,
                "discovery-contract-v1",
                &discovery_source(1, 'f'),
                &outcomes(true)
            )
            .unwrap(),
            analysis_id(
                &second,
                "discovery-contract-v1",
                &discovery_source(999, 'f'),
                &outcomes(false)
            )
            .unwrap()
        );

        let mut known_only = first.clone();
        known_only.known_artifacts.reverse();
        assert_eq!(
            analysis_id(&first, "v1", &discovery_source(1, 'f'), &outcomes(true)).unwrap(),
            analysis_id(
                &known_only,
                "v1",
                &discovery_source(1, 'f'),
                &outcomes(true)
            )
            .unwrap()
        );
        let mut proposals_only = first.clone();
        proposals_only.proposals.reverse();
        assert_eq!(
            analysis_id(&first, "v1", &discovery_source(1, 'f'), &outcomes(true)).unwrap(),
            analysis_id(
                &proposals_only,
                "v1",
                &discovery_source(1, 'f'),
                &outcomes(true)
            )
            .unwrap()
        );

        let evidence_a = evidence_id(&record("CommonModule.Flow.Run", "defs", 1)).unwrap();
        let evidence_b = evidence_id(&record("CommonModule.Flow.Run", "defs", 999)).unwrap();
        assert_eq!(evidence_a, evidence_b);
        assert_eq!(evidence_a.len(), 67);
        assert!(evidence_a.starts_with("ev_"));

        let first = request(json!({
            "mode":"validate","task":"x","concepts":["x"],
            "knownArtifacts":[
                {"kind":"module","ref":"CommonModule.B"},
                {"kind":"module","ref":"CommonModule.A"}
            ],
            "proposals":[
                {"id":"b","target":{"kind":"method","ref":"CommonModule.B.Run"},"intent":"b"},
                {"id":"a","target":{"kind":"method","ref":"CommonModule.A.Run"},"intent":"a"}
            ]
        }))
        .unwrap();
        let second = request(json!({
            "mode":"validate","task":"x","concepts":["x"],
            "knownArtifacts":[
                {"kind":"module","ref":"CommonModule.A"},
                {"kind":"module","ref":"CommonModule.B"}
            ],
            "proposals":[
                {"id":"a","target":{"kind":"method","ref":"CommonModule.A.Run"},"intent":"a"},
                {"id":"b","target":{"kind":"method","ref":"CommonModule.B.Run"},"intent":"b"}
            ]
        }))
        .unwrap();
        assert_eq!(
            analysis_id(&first, "v1", &discovery_source(1, 'f'), &outcomes(true)).unwrap(),
            analysis_id(&second, "v1", &discovery_source(1, 'f'), &outcomes(true)).unwrap()
        );
    }

    #[test]
    fn stable_ids_change_for_every_bound_identity_component() {
        let base = request(minimal_explore()).unwrap();
        let source = discovery_source(1, 'f');
        let base_id = analysis_id(&base, "v1", &source, &outcomes(true)).unwrap();

        let changed_request =
            request(json!({"mode":"explore","task":"different","concepts":["write"]})).unwrap();
        assert_ne!(
            base_id,
            analysis_id(&changed_request, "v1", &source, &outcomes(true)).unwrap()
        );
        assert_ne!(
            base_id,
            analysis_id(&base, "v2", &source, &outcomes(true)).unwrap()
        );
        assert_ne!(
            base_id,
            analysis_id(&base, "v1", &discovery_source(1, 'e'), &outcomes(true)).unwrap()
        );

        let mut changed_linked_only = source.clone();
        changed_linked_only.linked_source_snapshots[0].source_fingerprint =
            format!("sha256:{}", "e".repeat(64));
        assert_eq!(
            changed_linked_only.composite_source_fingerprint,
            source.composite_source_fingerprint
        );
        assert_ne!(
            base_id,
            analysis_id(&base, "v1", &changed_linked_only, &outcomes(true)).unwrap()
        );

        let mut changed_composite_only = source.clone();
        changed_composite_only.composite_source_fingerprint = format!("sha256:{}", "e".repeat(64));
        assert_eq!(
            changed_composite_only.linked_source_snapshots,
            source.linked_source_snapshots
        );
        assert_ne!(
            base_id,
            analysis_id(&base, "v1", &changed_composite_only, &outcomes(true)).unwrap()
        );

        let mut changed_limits = base.clone();
        changed_limits.limits.max_evidence += 1;
        assert_ne!(
            base_id,
            analysis_id(&changed_limits, "v1", &source, &outcomes(true)).unwrap()
        );

        let mut changed_provider = outcomes(true);
        changed_provider[0].name = "other".to_string();
        assert_ne!(
            base_id,
            analysis_id(&base, "v1", &source, &changed_provider).unwrap()
        );
        let mut changed_version = outcomes(true);
        changed_version[0].version = "2".to_string();
        assert_ne!(
            base_id,
            analysis_id(&base, "v1", &source, &changed_version).unwrap()
        );
        let mut changed_outcome = outcomes(true);
        changed_outcome[0].record_digests[0] = format!("sha256:{}", "b".repeat(64));
        assert_ne!(
            base_id,
            analysis_id(&base, "v1", &source, &changed_outcome).unwrap()
        );

        let base_evidence = record("CommonModule.Flow.Run", "defs", 1);
        let mut changed = base_evidence.clone();
        changed.provider.name = "defs-2".to_string();
        assert_ne!(
            evidence_id(&base_evidence).unwrap(),
            evidence_id(&changed).unwrap()
        );
        let mut changed = base_evidence.clone();
        changed.coverage = Coverage::Bounded;
        assert_ne!(
            evidence_id(&base_evidence).unwrap(),
            evidence_id(&changed).unwrap()
        );
        let mut changed = base_evidence.clone();
        changed.freshness.source_fingerprint = format!("sha256:{}", "2".repeat(64));
        assert_ne!(
            evidence_id(&base_evidence).unwrap(),
            evidence_id(&changed).unwrap()
        );
    }

    #[test]
    fn canonical_encoder_has_a_fixed_golden_vector_and_unique_stable_tags() {
        assert_eq!(
            canonical_golden_vector(),
            "29644b1c6e3e6097d8359c6fc1e1802593063854bf05bd9997930cb88c304f4f"
        );
        assert_unique_stable_tags();
    }

    #[test]
    fn provider_fact_discriminants_have_exhaustive_frozen_tags() {
        let method = ArtifactRef::parse(ArtifactKind::Method, "CommonModule.Flow.Run").unwrap();
        let module = ArtifactRef::parse(ArtifactKind::Module, "CommonModule.Flow").unwrap();
        let definition = DefinitionShape::new(false, true, vec![]).unwrap();
        let callback = PlatformCallbackShape::new(
            "8.3.24",
            "Document",
            "ObjectModule",
            "BeforeWrite",
            true,
            vec![],
        )
        .unwrap();
        let facts = [
            ProviderFact::MetadataPresent {
                subject: module.clone(),
            },
            ProviderFact::MetadataAbsent {
                subject: module.clone(),
            },
            ProviderFact::CodeOccurrence {
                subject: method.clone(),
                search_term: "Run".into(),
            },
            ProviderFact::DefinitionPresent {
                subject: method.clone(),
                definition,
            },
            ProviderFact::DefinitionAbsent {
                subject: method.clone(),
            },
            ProviderFact::Binding {
                subject: module.clone(),
                object: method.clone(),
                relation: FlowKind::Defines,
                details: BindingDetails::Structural,
            },
            ProviderFact::Call {
                subject: method.clone(),
                object: method.clone(),
                resolution: CallResolution::Resolved,
                call_type: CallType::Direct,
                context: ExecutionContext::Server,
            },
            ProviderFact::PlatformCallback {
                subject: module.clone(),
                object: method.clone(),
                callback,
            },
            ProviderFact::Support {
                subject: method,
                state: SupportState::Editable,
            },
        ];
        assert_eq!(
            facts.map(|fact| fact.stable_tag()),
            ProviderFact::VARIANT_STABLE_TAGS
        );
    }

    #[test]
    fn evidence_collapse_requires_identical_complete_payload() {
        let a = record("CommonModule.Flow.Run", "defs", 1);
        let duplicate = a.clone();
        let collapsed = canonicalize_evidence(vec![a.clone(), duplicate]).unwrap();
        assert_eq!(collapsed.len(), 1);

        let mut different = a.clone();
        different.provider.name = "other".to_string();
        let result =
            canonicalize_evidence_with_id(vec![a, different], |_| "ev_collision".to_string());
        assert!(matches!(
            result,
            Err(DeterminismError::IdentifierCollision { .. })
        ));

        let first = record("CommonModule.Flow.Run", "defs", 1);
        let second = record("CommonModule.Flow.Stop", "defs", 1);
        assert_eq!(
            canonicalize_evidence(vec![first.clone(), second.clone()]).unwrap(),
            canonicalize_evidence(vec![second, first]).unwrap()
        );
    }

    #[test]
    fn typed_fact_payload_and_relation_are_bound_into_record_digest() {
        let method = ArtifactRef::parse(ArtifactKind::Method, "CommonModule.Flow.Run").unwrap();
        let caller = ArtifactRef::parse(ArtifactKind::Method, "CommonModule.Flow.Start").unwrap();
        let provider = EvidenceProvider::new(EvidencePort::CallGraph, "graph", "1").unwrap();
        let freshness = Freshness::new("main", &format!("sha256:{}", "1".repeat(64)), 1).unwrap();
        let make = |relation| {
            EvidenceRecord::from_fact(
                ProviderFact::Binding {
                    subject: caller.clone(),
                    object: method.clone(),
                    relation,
                    details: BindingDetails::Structural,
                },
                None,
                provider.clone(),
                Coverage::Complete,
                freshness.clone(),
            )
        };
        assert_ne!(
            evidence_record_digest(&make(FlowKind::Calls)).unwrap(),
            evidence_record_digest(&make(FlowKind::Handles)).unwrap()
        );

        let binding = |url_template: &str| {
            EvidenceRecord::from_fact(
                ProviderFact::Binding {
                    subject: caller.clone(),
                    object: method.clone(),
                    relation: FlowKind::Handles,
                    details: BindingDetails::HttpRoute {
                        verb: HttpVerb::Post,
                        url_template: url_template.to_string(),
                        context: ExecutionContext::Server,
                    },
                },
                None,
                provider.clone(),
                Coverage::Complete,
                freshness.clone(),
            )
        };
        assert_ne!(
            evidence_record_digest(&binding("/v1/items/{id}")).unwrap(),
            evidence_record_digest(&binding("/v1/items/{code}")).unwrap()
        );

        let call = |resolution, context| {
            EvidenceRecord::from_fact(
                ProviderFact::Call {
                    subject: caller.clone(),
                    object: method.clone(),
                    resolution,
                    call_type: CallType::Direct,
                    context,
                },
                None,
                provider.clone(),
                Coverage::Complete,
                freshness.clone(),
            )
        };
        assert_ne!(
            evidence_record_digest(&call(CallResolution::Resolved, ExecutionContext::Server))
                .unwrap(),
            evidence_record_digest(&call(CallResolution::Dynamic, ExecutionContext::Server))
                .unwrap()
        );
        assert_ne!(
            evidence_record_digest(&call(CallResolution::Resolved, ExecutionContext::Server))
                .unwrap(),
            evidence_record_digest(&call(CallResolution::Resolved, ExecutionContext::Client))
                .unwrap()
        );

        let definition_provider =
            EvidenceProvider::new(EvidencePort::Definition, "defs", "1").unwrap();
        let definition = |exported| {
            EvidenceRecord::from_fact(
                ProviderFact::DefinitionPresent {
                    subject: method.clone(),
                    definition: DefinitionShape::new(
                        false,
                        exported,
                        vec![DefinitionParameter::new("Value", true, false).unwrap()],
                    )
                    .unwrap(),
                },
                None,
                definition_provider.clone(),
                Coverage::Complete,
                freshness.clone(),
            )
        };
        assert_ne!(
            evidence_id(&definition(false)).unwrap(),
            evidence_id(&definition(true)).unwrap()
        );

        let callback_provider =
            EvidenceProvider::new(EvidencePort::MetadataCatalog, "callbacks", "1").unwrap();
        let callback = |variant| {
            EvidenceRecord::from_fact(
                ProviderFact::PlatformCallback {
                    subject: method.clone(),
                    object: caller.clone(),
                    callback: PlatformCallbackShape::new(
                        variant,
                        "Document",
                        "ObjectModule",
                        "BeforeWrite",
                        true,
                        vec![DefinitionParameter::new("Refusal", false, false).unwrap()],
                    )
                    .unwrap(),
                },
                None,
                callback_provider.clone(),
                Coverage::Complete,
                freshness.clone(),
            )
        };
        assert_ne!(
            evidence_id(&callback("8.3.24")).unwrap(),
            evidence_id(&callback("8.3.25")).unwrap()
        );
        assert!(evidence_record_digest(&callback("8.3.24"))
            .unwrap()
            .starts_with("sha256:"));
    }

    #[test]
    fn provider_outcome_snapshot_rejects_impossible_state_combinations() {
        let digest = format!("sha256:{}", "a".repeat(64));
        assert!(ProviderOutcomeSnapshot::new(
            EvidencePort::Definition,
            "defs",
            "1",
            ProviderReadiness::Ready,
            Coverage::Unknown,
            None,
            vec![]
        )
        .is_err());
        assert!(ProviderOutcomeSnapshot::new(
            EvidencePort::Definition,
            "defs",
            "1",
            ProviderReadiness::Unavailable,
            Coverage::Complete,
            Some("offline".into()),
            vec![]
        )
        .is_err());
        assert!(ProviderOutcomeSnapshot::new(
            EvidencePort::Definition,
            "defs",
            "1",
            ProviderReadiness::Failed,
            Coverage::Unknown,
            None,
            vec![]
        )
        .is_err());
        assert!(ProviderOutcomeSnapshot::new(
            EvidencePort::Definition,
            "defs",
            "1",
            ProviderReadiness::Unavailable,
            Coverage::Unknown,
            Some("offline".into()),
            vec![digest]
        )
        .is_err());

        let records = vec![record("CommonModule.Flow.Run", "defs", 1)];
        let snapshot = ProviderOutcomeSnapshot::from_records(
            EvidencePort::Definition,
            "defs",
            "1",
            Coverage::Complete,
            None,
            &records,
        )
        .unwrap();
        assert_eq!(snapshot.readiness, ProviderReadiness::Ready);
        assert_eq!(
            snapshot.record_digests,
            vec![evidence_record_digest(&records[0]).unwrap()]
        );
    }

    #[test]
    fn production_check_constructor_applies_the_same_strict_validation() {
        let check = Check::new(
            "call_graph",
            "CallGraphPort",
            CheckState::Unavailable,
            CheckOutcome::Inconclusive,
            Coverage::Unknown,
            CheckSeverity::Blocking,
            vec!["proposal:p".into()],
            "index_building",
            true,
            vec!["workspace index is building".into()],
            vec![],
        )
        .unwrap();
        assert!(check.validate().is_ok());
        assert!(Check::new(
            "call_graph",
            "CallGraphPort",
            CheckState::Unavailable,
            CheckOutcome::Inconclusive,
            Coverage::Unknown,
            CheckSeverity::Blocking,
            vec![],
            "index_building",
            true,
            vec!["x".repeat(513)],
            vec![],
        )
        .is_err());
        assert!(Check::new(
            "source_readiness",
            "DefinitionPort",
            CheckState::Skipped,
            CheckOutcome::Inconclusive,
            Coverage::Unknown,
            CheckSeverity::Blocking,
            vec![],
            "unsupported_source_format",
            false,
            vec![],
            vec![],
        )
        .is_err());
    }

    #[test]
    fn checks_accept_evidence_ports_and_the_source_resolver_orchestration_port() {
        for port in EvidencePort::ALL {
            let name = port.wire_name();
            assert_eq!(EvidencePort::parse_wire_name(name), Some(port));
            assert!(Check::new(
                "provider_contract",
                name,
                CheckState::Passed,
                CheckOutcome::Satisfied,
                Coverage::Complete,
                CheckSeverity::Info,
                vec![],
                "ok",
                false,
                vec![],
                vec![],
            )
            .is_ok());
        }
        assert!(Check::new(
            "source_readiness",
            "ProjectSourceResolverPort",
            CheckState::Skipped,
            CheckOutcome::Inconclusive,
            Coverage::Unknown,
            CheckSeverity::Blocking,
            vec![],
            "unsupported_source_format",
            false,
            vec![],
            vec![],
        )
        .is_ok());
        assert!(Check::new(
            "provider_contract",
            "ProjectSourceResolverPort",
            CheckState::Skipped,
            CheckOutcome::Inconclusive,
            Coverage::Unknown,
            CheckSeverity::Blocking,
            vec![],
            "unsupported_source_format",
            false,
            vec![],
            vec![],
        )
        .is_err());
        assert!(Check::new(
            "source_readiness",
            "ProjectSourceResolverPort",
            CheckState::Failed,
            CheckOutcome::Inconclusive,
            Coverage::Unknown,
            CheckSeverity::Blocking,
            vec!["proposal:p".into()],
            "unsupported_source_format",
            false,
            vec![],
            vec![],
        )
        .is_err());
        assert!(Check::new(
            "source_readiness",
            "ProjectSourceResolverPort",
            CheckState::Skipped,
            CheckOutcome::Inconclusive,
            Coverage::Unknown,
            CheckSeverity::Blocking,
            vec!["candidate:not-allowed".into()],
            "unsupported_source_format",
            false,
            vec![],
            vec![],
        )
        .is_err());
        for provider in ["SyntheticProvider", "definitionport", "DefinitionPortTypo"] {
            assert!(Check::new(
                "provider_contract",
                provider,
                CheckState::Passed,
                CheckOutcome::Satisfied,
                Coverage::Complete,
                CheckSeverity::Info,
                vec![],
                "ok",
                false,
                vec![],
                vec![],
            )
            .is_err());
        }
    }

    fn valid_report() -> DiscoveryReport {
        let method = ArtifactRef::parse(ArtifactKind::Method, "CommonModule.Flow.Run").unwrap();
        let module = ArtifactRef::parse(ArtifactKind::Module, "CommonModule.Flow").unwrap();
        let evidence =
            canonicalize_evidence(vec![record("CommonModule.Flow.Run", "defs", 1)]).unwrap();
        let evidence_id = evidence[0].id.clone();
        DiscoveryReport::new(
            DiscoveryStatus::Complete,
            format!("analysis_{}", "a".repeat(64)),
            DiscoverySource {
                analysis_source_set: "main".into(),
                source_format: SourceFormat::PlatformXml,
                workspace_epoch: 1,
                linked_source_snapshots: vec![LinkedSourceSnapshot {
                    source_set: "main".into(),
                    role: SourceSnapshotRole::Analysis,
                    source_fingerprint: format!("sha256:{}", "1".repeat(64)),
                }],
                composite_source_fingerprint: format!("sha256:{}", "2".repeat(64)),
            },
            vec![RelatedArtifact {
                artifact: method.clone(),
                evidence_level: EvidenceLevel::Actionable,
                reason_codes: vec!["definition_exists".into()],
                evidence_ids: vec![evidence_id.clone()],
            }],
            vec![FlowEdge {
                from: module,
                to: method.clone(),
                kind: FlowKind::Defines,
                evidence_ids: vec![evidence_id.clone()],
            }],
            vec![Candidate {
                target: method,
                evidence_level: EvidenceLevel::Actionable,
                support_state: SupportState::Editable,
                reason_codes: vec!["reachable".into()],
                evidence_ids: vec![evidence_id.clone()],
                blockers: vec![],
            }],
            vec![ProposalVerdict {
                proposal_id: "p".into(),
                verdict: Verdict::Supported,
                facts: ProposalFacts {
                    exists: FactAnswer::Yes,
                    runtime_reachable: FactAnswer::Yes,
                    support: SupportState::Editable,
                },
                evidence_ids: vec![evidence_id.clone()],
                coverage_gaps: vec![],
                blockers: vec![],
            }],
            evidence,
            vec![Check::new(
                "definition",
                "DefinitionPort",
                CheckState::Passed,
                CheckOutcome::Satisfied,
                Coverage::Complete,
                CheckSeverity::Info,
                vec!["proposal:p".into()],
                "ok",
                false,
                vec![],
                vec![evidence_id],
            )
            .unwrap()],
            ReceiptEligibility {
                eligible: true,
                blockers: vec![],
            },
        )
        .unwrap()
    }

    #[test]
    fn discovery_report_validates_canonicalizes_and_round_trips() {
        let report = valid_report();
        assert!(report.validate().is_ok());
        let encoded = serde_json::to_value(&report).unwrap();
        assert_eq!(encoded["schemaVersion"], 1);
        let decoded: DiscoveryReport = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, report);
    }

    #[test]
    fn discovery_report_rejects_invalid_ids_fingerprints_sources_and_artifacts() {
        let mut cases = Vec::new();

        let mut report = valid_report();
        report.schema_version = 2;
        cases.push(report);
        let mut report = valid_report();
        report.analysis_id = format!("analysis_{}", "A".repeat(64));
        cases.push(report);
        let mut report = valid_report();
        report.evidence[0].id = format!("ev_{}", "A".repeat(64));
        cases.push(report);
        let mut report = valid_report();
        report.source.composite_source_fingerprint = format!("sha256:{}", "A".repeat(64));
        cases.push(report);
        let mut report = valid_report();
        report.source.analysis_source_set = " ".into();
        cases.push(report);
        let mut report = valid_report();
        report.source.linked_source_snapshots[0].source_set = "".into();
        cases.push(report);
        let mut report = valid_report();
        report.evidence[0].provider.name = "\n".into();
        cases.push(report);
        let mut report = valid_report();
        report.related_artifacts[0].artifact.canonical_ref = "../outside".into();
        cases.push(report);
        let mut report = valid_report();
        report.evidence[0].subject.canonical_ref = "CommonModule..Run".into();
        cases.push(report);

        for report in cases {
            assert!(report.validate().is_err());
            assert!(serde_json::to_value(&report).is_err());
        }
    }

    #[test]
    fn discovery_report_rejects_evidence_collisions_and_dangling_references() {
        let mut identical = valid_report();
        identical.evidence.push(identical.evidence[0].clone());
        identical.canonicalize().unwrap();
        assert_eq!(identical.evidence.len(), 1);

        let mut collision = valid_report();
        let mut different_payload = collision.evidence[0].clone();
        different_payload.subject =
            ArtifactRef::parse(ArtifactKind::Method, "CommonModule.Flow.Stop").unwrap();
        collision.evidence.push(different_payload);
        assert!(collision.validate().is_err());
        assert!(serde_json::to_value(&collision).is_err());

        let missing = format!("ev_{}", "f".repeat(64));
        let mut cases = Vec::new();
        let mut report = valid_report();
        report.related_artifacts[0].evidence_ids = vec![missing.clone()];
        cases.push(report);
        let mut report = valid_report();
        report.flow_edges[0].evidence_ids = vec![missing.clone()];
        cases.push(report);
        let mut report = valid_report();
        report.extension_point_candidates[0].evidence_ids = vec![missing.clone()];
        cases.push(report);
        let mut report = valid_report();
        report.proposal_verdicts[0].evidence_ids = vec![missing.clone()];
        cases.push(report);
        let mut report = valid_report();
        report.checks[0].evidence_ids = vec![missing];
        cases.push(report);
        for report in cases {
            assert!(report.validate().is_err());
        }
    }

    #[test]
    fn discovery_report_rejects_duplicate_verdicts_bad_codes_and_cross_links() {
        let mut duplicate = valid_report();
        let mut conflicting = duplicate.proposal_verdicts[0].clone();
        conflicting.verdict = Verdict::Unknown;
        conflicting.facts.runtime_reachable = FactAnswer::Unknown;
        conflicting.coverage_gaps = vec!["graph_unavailable".into()];
        duplicate.proposal_verdicts.push(conflicting);
        assert!(duplicate.validate().is_err());

        let mut bad_affect = valid_report();
        bad_affect.checks[0].affects = vec!["proposal:missing".into()];
        assert!(bad_affect.validate().is_err());
        let mut bad_candidate = valid_report();
        bad_candidate.checks[0].affects = vec!["candidate:CommonModule.Flow.Stop".into()];
        assert!(bad_candidate.validate().is_err());
        let mut bad_prefix = valid_report();
        bad_prefix.checks[0].affects = vec!["artifact:CommonModule.Flow.Run".into()];
        assert!(bad_prefix.validate().is_err());

        let mut cases = Vec::new();
        let mut report = valid_report();
        report.evidence[0].fact_code = "Not Stable".into();
        cases.push(report);
        let mut report = valid_report();
        report.related_artifacts[0].reason_codes = vec!["Not Stable".into()];
        cases.push(report);
        let mut report = valid_report();
        report.extension_point_candidates[0].reason_codes = vec!["Not Stable".into()];
        cases.push(report);
        let mut report = valid_report();
        report.extension_point_candidates[0].blockers = vec!["Not Stable".into()];
        cases.push(report);
        let mut report = valid_report();
        report.proposal_verdicts[0].coverage_gaps = vec!["Not Stable".into()];
        cases.push(report);
        let mut report = valid_report();
        report.proposal_verdicts[0].blockers = vec!["Not Stable".into()];
        cases.push(report);
        let mut report = valid_report();
        report.receipt_eligibility.blockers = vec!["Not Stable".into()];
        cases.push(report);
        for report in cases {
            assert!(report.validate().is_err());
        }
    }

    #[test]
    fn discovery_report_rejects_snapshot_and_freshness_inconsistency() {
        let mut freshness = valid_report();
        freshness.evidence[0].freshness.source_fingerprint = format!("sha256:{}", "9".repeat(64));
        assert!(freshness.validate().is_err());

        let mut wrong_analysis = valid_report();
        wrong_analysis.source.linked_source_snapshots[0].source_set = "other".into();
        assert!(wrong_analysis.validate().is_err());

        let mut duplicate_role = valid_report();
        duplicate_role
            .source
            .linked_source_snapshots
            .push(LinkedSourceSnapshot {
                source_set: "main".into(),
                role: SourceSnapshotRole::Analysis,
                source_fingerprint: format!("sha256:{}", "8".repeat(64)),
            });
        assert!(duplicate_role.validate().is_err());
    }

    #[test]
    fn report_allows_supported_conclusions_with_separate_non_material_blockers() {
        let mut report = valid_report();
        report.proposal_verdicts[0].facts.support = SupportState::Locked;
        report.proposal_verdicts[0].coverage_gaps = vec!["optional_search_unavailable".into()];
        report.proposal_verdicts[0].blockers = vec!["direct_mutation_locked".into()];
        report.receipt_eligibility = ReceiptEligibility {
            eligible: false,
            blockers: vec!["direct_mutation_locked".into()],
        };
        assert!(report.validate().is_ok());
        assert!(serde_json::to_value(report).is_ok());
    }

    #[test]
    fn matching_content_freshness_may_have_an_older_diagnostic_epoch() {
        let mut report = valid_report();
        report.evidence[0].freshness.workspace_epoch = 0;
        assert!(report.validate().is_ok());
    }

    #[test]
    fn canonical_report_order_covers_every_public_collection() {
        let method_a = ArtifactRef::parse(ArtifactKind::Method, "CommonModule.Flow.A").unwrap();
        let method_b = ArtifactRef::parse(ArtifactKind::Method, "CommonModule.Flow.B").unwrap();
        let module = ArtifactRef::parse(ArtifactKind::Module, "CommonModule.Flow").unwrap();
        let evidence = canonicalize_evidence(vec![
            record("CommonModule.Flow.B", "defs", 1),
            record("CommonModule.Flow.A", "defs", 1),
        ])
        .unwrap();
        let check_a: Check = serde_json::from_value(json!({
            "code":"a","provider":"DefinitionPort","state":"passed","outcome":"satisfied",
            "coverage":"complete","severity":"info","affects":["proposal:b","proposal:a","proposal:a"],
            "reasonCode":"ok","retryable":false,"details":["z","a","a"],
            "evidenceIds":[evidence[1].id.clone(),evidence[0].id.clone(),evidence[0].id.clone()]
        })).unwrap();
        let check_b: Check = serde_json::from_value(json!({
            "code":"b","provider":"CallGraphPort","state":"unavailable","outcome":"inconclusive",
            "coverage":"unknown","severity":"blocking","affects":["proposal:b"],
            "reasonCode":"index_building","retryable":true,"details":["later"],"evidenceIds":[]
        }))
        .unwrap();
        let check_same_primary: Check = serde_json::from_value(json!({
            "code":"a","provider":"DefinitionPort","state":"passed","outcome":"satisfied",
            "coverage":"complete","severity":"info","affects":["proposal:c"],
            "reasonCode":"ok","retryable":false,"details":["different"],"evidenceIds":[]
        }))
        .unwrap();
        let mut first = DiscoveryReport {
            schema_version: 1,
            status: DiscoveryStatus::Complete,
            analysis_id: format!("analysis_{}", "a".repeat(64)),
            source: DiscoverySource {
                analysis_source_set: "main".to_string(),
                source_format: SourceFormat::PlatformXml,
                workspace_epoch: 1,
                linked_source_snapshots: vec![
                    LinkedSourceSnapshot {
                        source_set: "extension".to_string(),
                        role: SourceSnapshotRole::Mutation,
                        source_fingerprint: format!("sha256:{}", "2".repeat(64)),
                    },
                    LinkedSourceSnapshot {
                        source_set: "main".to_string(),
                        role: SourceSnapshotRole::Analysis,
                        source_fingerprint: format!("sha256:{}", "1".repeat(64)),
                    },
                ],
                composite_source_fingerprint: format!("sha256:{}", "3".repeat(64)),
            },
            related_artifacts: vec![
                RelatedArtifact {
                    artifact: method_b.clone(),
                    evidence_level: EvidenceLevel::Observed,
                    reason_codes: vec!["z".into(), "a".into(), "a".into()],
                    evidence_ids: vec![evidence[1].id.clone(), evidence[0].id.clone()],
                },
                RelatedArtifact {
                    artifact: method_a.clone(),
                    evidence_level: EvidenceLevel::Connected,
                    reason_codes: vec!["b".into(), "a".into()],
                    evidence_ids: vec![],
                },
                RelatedArtifact {
                    artifact: method_b.clone(),
                    evidence_level: EvidenceLevel::Observed,
                    reason_codes: vec!["same_primary_different_payload".into()],
                    evidence_ids: vec![],
                },
            ],
            flow_edges: vec![
                FlowEdge {
                    from: module.clone(),
                    to: method_b.clone(),
                    kind: FlowKind::Defines,
                    evidence_ids: vec![evidence[1].id.clone(), evidence[1].id.clone()],
                },
                FlowEdge {
                    from: module,
                    to: method_a.clone(),
                    kind: FlowKind::Defines,
                    evidence_ids: vec![evidence[0].id.clone()],
                },
            ],
            extension_point_candidates: vec![
                Candidate {
                    target: method_b.clone(),
                    evidence_level: EvidenceLevel::Actionable,
                    support_state: SupportState::Editable,
                    reason_codes: vec!["z".into(), "a".into()],
                    evidence_ids: vec![evidence[1].id.clone(), evidence[1].id.clone()],
                    blockers: vec!["z".into(), "a".into()],
                },
                Candidate {
                    target: method_a,
                    evidence_level: EvidenceLevel::Connected,
                    support_state: SupportState::Unknown,
                    reason_codes: vec![],
                    evidence_ids: vec![],
                    blockers: vec![],
                },
            ],
            proposal_verdicts: vec![
                ProposalVerdict {
                    proposal_id: "b".into(),
                    verdict: Verdict::Unknown,
                    facts: ProposalFacts {
                        exists: FactAnswer::Yes,
                        runtime_reachable: FactAnswer::Unknown,
                        support: SupportState::Unknown,
                    },
                    evidence_ids: vec![evidence[1].id.clone(), evidence[1].id.clone()],
                    coverage_gaps: vec!["z".into(), "a".into()],
                    blockers: vec!["z".into(), "a".into()],
                },
                ProposalVerdict {
                    proposal_id: "a".into(),
                    verdict: Verdict::Supported,
                    facts: ProposalFacts {
                        exists: FactAnswer::Yes,
                        runtime_reachable: FactAnswer::Yes,
                        support: SupportState::Editable,
                    },
                    evidence_ids: vec![],
                    coverage_gaps: vec![],
                    blockers: vec![],
                },
            ],
            evidence,
            checks: vec![check_b, check_same_primary, check_a],
            receipt_eligibility: ReceiptEligibility {
                eligible: false,
                blockers: vec!["z".into(), "a".into(), "a".into()],
            },
        };
        let mut second = first.clone();
        second.source.linked_source_snapshots.reverse();
        second.related_artifacts.reverse();
        for item in &mut second.related_artifacts {
            item.reason_codes.reverse();
            item.evidence_ids.reverse();
        }
        second.flow_edges.reverse();
        for item in &mut second.flow_edges {
            item.evidence_ids.reverse();
        }
        second.extension_point_candidates.reverse();
        for item in &mut second.extension_point_candidates {
            item.reason_codes.reverse();
            item.evidence_ids.reverse();
            item.blockers.reverse();
        }
        second.proposal_verdicts.reverse();
        for item in &mut second.proposal_verdicts {
            item.evidence_ids.reverse();
            item.coverage_gaps.reverse();
            item.blockers.reverse();
        }
        second.evidence.reverse();
        second.checks.reverse();
        for item in &mut second.checks {
            item.affects.reverse();
            item.details.reverse();
            item.evidence_ids.reverse();
        }
        second.receipt_eligibility.blockers.reverse();

        canonicalize_report(&mut first);
        canonicalize_report(&mut second);
        assert_eq!(first, second);
        assert_eq!(first.receipt_eligibility.blockers, ["a", "z"]);
        assert_eq!(first.related_artifacts[1].reason_codes, ["a", "z"]);
    }
}
