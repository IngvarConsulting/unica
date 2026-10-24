//! Existing cross-component canonical metadata tests.
use super::validate_platform_xml_binding;
use serde_json::Value;
use unica_address::address::QualifiedAddress;
use unica_metadata_designer::decode::*;
use unica_metadata_designer::planner::set_ml_property;
use unica_operations::OperationDescriptor;
use unica_source_contract::proposal::{
    ApplyPlanError, IndexedPlanOperation, ProvisionalApplyEffect,
};
use unica_workspace::actor::ProviderRootBinding;
#[cfg(test)]
fn parse_metadata_plan_operation_in(
    descriptor: Option<OperationDescriptor>,
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, operation, op_index)?;
    unica_metadata_designer::decode::parse_metadata_plan_operation_in(
        descriptor,
        operation,
        args,
        op_index,
        &MetadataDecodeContext {
            source_set_name: binding.source_set_name(),
            registry: crate::domain::apply::operation_registry(),
        },
    )
}

#[cfg(test)]
pub(crate) fn parse_metadata_plan_operation(
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanOperation, ApplyPlanError> {
    let at = args
        .get("at")
        .and_then(Value::as_str)
        .and_then(|raw| QualifiedAddress::resolve_input(raw, &[binding.source_set_name()]).ok());
    let descriptor = super::resolved_descriptor(operation, at.as_ref(), binding);
    parse_metadata_plan_operation_in(descriptor, operation, args, op_index, binding)
}

#[cfg(test)]
pub(crate) fn plan_metadata_batch_owned_for_test(
    mut staged: unica_workspace::apply::ApplyStagedState,
    authority: unica_workspace::actor::MetadataApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<MetadataPlanOperation>],
) -> Result<
    (
        unica_workspace::apply::ApplyStagedState,
        Vec<ProvisionalApplyEffect>,
    ),
    ApplyPlanError,
> {
    let effects = unica_metadata_designer::planner::plan_metadata_batch(&crate::infrastructure::native_operations::meta::template_source::MetadataPlanningReadAdapter::new(&authority),unica_metadata_designer::planner::TemplateSeedEmitter(unica_templates_designer::mxl_root::empty_spreadsheet_document_xml), &mut staged.proposal(), &authority, operations)?;
    Ok((staged, effects))
}

#[cfg(test)]
mod tests {
    mod contract_shape_tests {
        include!("metadata/contract_shape_tests.rs");
    }

    use super::{
        parse_metadata_plan_operation, plan_metadata_batch_owned_for_test, set_ml_property,
    };
    use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};

    use crate::infrastructure::source_workspace::WorkspaceActor;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;
    use unica_source_contract::proposal::IndexedPlanOperation;
    use unica_source_contract::proposal::{ApplyPlanErrorKind, StagedChangeKind, StagedFileState};
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_work_control::cancellation::CancellationToken;
    use unica_work_control::provider_deadline::ProviderDeadline;
    use unica_workspace::actor::{
        ApplyAdmission, ProviderRootBinding, WorkspaceIdentity, WorkspaceSourceSetInput,
    };

    const ORDER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Document uuid="11111111-1111-4111-8111-111111111111">
		<Properties><Name>Order</Name><Synonym/><Comment/><BasedOn/></Properties>
		<ChildObjects/>
	</Document>
</MetaDataObject>
"#;

    struct MetadataFixture {
        _root: tempfile::TempDir,
        actor: Arc<WorkspaceActor>,
        binding: ProviderRootBinding,
        descriptor: PathBuf,
    }

    const RU_CONFIGURATION_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#;

    fn language_xml(name: &str, code: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>{name}</Name><Comment/><LanguageCode>{code}</LanguageCode></Properties></Language></MetaDataObject>"#
        )
    }

    impl MetadataFixture {
        fn new() -> Self {
            Self::with_files(&[
                ("Configuration.xml", RU_CONFIGURATION_XML),
                ("Languages/Русский.xml", &language_xml("Русский", "ru")),
                ("Documents/Order.xml", ORDER_XML),
            ])
        }

        /// A `main` configuration source set made of `files` (relative
        /// path, text); `descriptor` is `Documents/Order.xml`.
        fn with_files(files: &[(&str, &str)]) -> Self {
            Self::with_kind_and_files(SourceSetKind::Configuration, files)
        }

        fn with_kind_and_files(kind: SourceSetKind, files: &[(&str, &str)]) -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("src");
            std::fs::create_dir_all(&source).unwrap();
            let project_kind = match kind {
                SourceSetKind::Configuration => "CONFIGURATION",
                SourceSetKind::Extension => "EXTENSION",
                other => panic!("unsupported fixture kind {other:?}"),
            };
            std::fs::write(
                root.path().join("v8project.yaml"),
                format!("format: DESIGNER\nsource-set:\n  - name: main\n    type: {project_kind}\n    path: src\n"),
            )
            .unwrap();
            for (relative, text) in files {
                let path = source.join(relative);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            let descriptor = source.join("Documents/Order.xml");
            let workspace_root = std::fs::canonicalize(root.path()).unwrap();
            let source = std::fs::canonicalize(source).unwrap();
            let context = WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            };
            let identity = WorkspaceIdentity::new(
                &context,
                [WorkspaceSourceSetInput::new(
                    "main",
                    &source,
                    kind,
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                )],
                "metadata-family-planner-test",
            )
            .unwrap();
            let actor = Arc::new(
                crate::infrastructure::source_workspace::new_actor(identity, context).unwrap(),
            );
            let binding = actor.bind_provider_root("main", &source).unwrap();
            Self {
                _root: root,
                actor,
                binding,
                descriptor,
            }
        }

        fn admission(&self) -> ApplyAdmission {
            self.actor
                .admit_apply(
                    &self.binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
        }

        fn parse(
            &self,
            operation: &str,
            args: serde_json::Value,
            index: usize,
        ) -> IndexedPlanOperation<super::MetadataPlanOperation> {
            IndexedPlanOperation::new(
                index,
                parse_metadata_plan_operation(operation, &args, index, &self.binding).unwrap(),
            )
        }

        fn disk_bytes(&self) -> Vec<u8> {
            std::fs::read(&self.descriptor).unwrap()
        }
    }

    #[test]
    fn the_first_new_item_name_stays_the_callers_field() {
        use super::AddressedFields;
        assert_eq!(
            AddressedFields::Items
                .caller_field("elements[0].name")
                .as_deref(),
            Some("items[0].name")
        );
        assert_eq!(
            AddressedFields::Items
                .caller_field("elements[1].name")
                .as_deref(),
            Some("items[1].name")
        );
        assert_eq!(
            AddressedFields::Values.caller_field("elements[0].name"),
            None
        );
        assert_eq!(AddressedFields::None.caller_field("elements[0].name"), None);
        assert_eq!(AddressedFields::Items.caller_field("names[0]"), None);
    }
    #[test]
    fn metadata_parser_rejects_unknown_and_misplaced_fields_at_the_exact_operation_path() {
        let fixture = MetadataFixture::new();
        let unknown = parse_metadata_plan_operation(
            "props.set",
            &json!({
                "at": "main:Document.Order",
                "values": {"Comment": "typed"},
                "command": "forbidden"
            }),
            3,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(unknown.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(unknown.path(), Some("ops[3].args.command"));

        let misplaced = parse_metadata_plan_operation(
            "attribute.remove",
            &json!({
                "at": "main:Document.Order",
                "items": ["Total"]
            }),
            4,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(misplaced.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(misplaced.path(), Some("ops[4].args.items"));
    }

    #[test]
    fn properties_unknown_envelope_names_values_before_decoding_the_payload() {
        let fixture = MetadataFixture::new();
        for at in [
            "main:Document.Order",
            "main:Role.Reader",
            "main:Subsystem.Sales",
        ] {
            for (mut args, field) in [
                (json!({"props":{"Comment":"wrong envelope"}}), "props"),
                (
                    json!({"values":false,"command":"wrong envelope"}),
                    "command",
                ),
            ] {
                args["at"] = json!(at);
                let error = parse_metadata_plan_operation("props.set", &args, 3, &fixture.binding)
                    .unwrap_err();
                assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{at}: {error}");
                let path = format!("ops[3].args.{field}");
                assert_eq!(error.path(), Some(path.as_str()), "{at}: {error}");
                assert_eq!(
                    error.to_string(),
                    format!(
                        "operation does not accept argument `{field}`; `props.set` expects `values`"
                    ),
                    "{at}"
                );
            }
        }
    }

    #[test]
    fn migrated_metadata_resources_keep_their_own_argument_stages() {
        let fixture = MetadataFixture::new();
        let before = fixture.disk_bytes();
        for (operation, args, path, message) in [
            (
                "template.add",
                json!({"at":"main:Document.Order.Attribute.Total", "items":false}),
                "ops[3].args.items",
                "expected \"array\"",
            ),
            (
                "predefinedItem.set",
                json!({"at":"main:Catalog.Items.PredefinedItem.Sample", "values":{"id":false,"name":false,"newName":false}}),
                "ops[3].args.values.id",
                "the predefined item is named by its address, not by `id`",
            ),
        ] {
            let descriptor = crate::domain::apply::operation_registry()
                .lookup(operation)
                .unwrap();
            let error = super::parse_metadata_plan_operation_in(
                Some(descriptor),
                operation,
                &args,
                3,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{operation}");
            assert_eq!(error.path(), Some(path), "{operation}");
            assert!(error.to_string().contains(message), "{operation}: {error}");
        }
        // This lower parser has always refused Borrow. Its real typed family
        // entry keeps the existing Borrow carrier and parent guards.
        let descriptor = crate::domain::apply::operation_registry()
            .lookup("object.borrow")
            .unwrap();
        for args in [json!({}), json!({"unknown":true})] {
            let error = super::parse_metadata_plan_operation_in(
                Some(descriptor),
                "object.borrow",
                &args,
                4,
                &fixture.binding,
            )
            .unwrap_err();
            let expected = unica_source_contract::proposal::hidden_apply_family_unimplemented(4);
            assert_eq!(error.kind(), expected.kind());
            assert_eq!(error.path(), expected.path());
            assert_eq!(error.to_string(), expected.to_string());
        }
        assert_eq!(fixture.disk_bytes(), before);
    }

    #[test]
    fn typed_member_additions_keep_the_owner_collection_and_section() {
        use super::{AddressedFields, MetadataPlanKind};
        use unica_metadata::{MetaCollection, MetaEditOperation};
        let fixture = MetadataFixture::new();
        for (operation, at, collection, section) in [
            (
                "attribute.add",
                "Document.Order",
                MetaCollection::Attributes,
                None,
            ),
            (
                "attribute.add",
                "main:Document.Order.TabularSection.Lines",
                MetaCollection::Attributes,
                Some("Lines"),
            ),
            (
                "tabularSection.add",
                "Document.Order",
                MetaCollection::TabularSections,
                None,
            ),
            (
                "dimension.add",
                "InformationRegister.Prices",
                MetaCollection::Dimensions,
                None,
            ),
            (
                "resource.add",
                "InformationRegister.Prices",
                MetaCollection::Resources,
                None,
            ),
            (
                "enumValue.add",
                "Enum.Kinds",
                MetaCollection::EnumValues,
                None,
            ),
            (
                "column.add",
                "DocumentJournal.All",
                MetaCollection::Columns,
                None,
            ),
            (
                "command.add",
                "Document.Order",
                MetaCollection::Commands,
                None,
            ),
        ] {
            let parsed = parse_metadata_plan_operation(
                operation,
                &json!({"at":at,"items":[{"name":"Added"}]}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let MetadataPlanKind::Edit {
                target,
                operation:
                    MetaEditOperation::Add {
                        collection: actual,
                        scope,
                        elements,
                    },
                addressed,
            } = parsed.kind
            else {
                panic!("{operation} must plan a typed metadata addition");
            };
            let owner = at.strip_prefix("main:").unwrap_or(at);
            let owner = owner.split(".TabularSection.").next().unwrap();
            assert_eq!(target.as_str(), owner);
            assert_eq!(actual, collection);
            assert_eq!(
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
                section
            );
            assert_eq!(elements.len(), 1);
            assert_eq!(elements[0].name, "Added");
            assert_eq!(addressed, section.map(|_| AddressedFields::Items));
        }
    }

    #[test]
    fn typed_metadata_additions_keep_writer_parser_refusal_order() {
        let fixture = MetadataFixture::new();
        for operation in ["attribute.add", "command.add"] {
            let error = parse_metadata_plan_operation(
                operation,
                &json!({"at":null,"items":false,"unknown":true}),
                3,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
            assert_eq!(error.path(), Some("ops[3].args.unknown"));
            assert_eq!(
                error.to_string(),
                format!(
                    "operation does not accept argument `unknown`; `{operation}` expects `items`"
                )
            );
            let error = parse_metadata_plan_operation(
                operation,
                &json!({"at":42,"items":false}),
                3,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.path(), Some("ops[3].args.at"));
            assert_eq!(error.to_string(), "`at` must be a logical address");
            let error = parse_metadata_plan_operation(
                operation,
                &json!({"at":"main:Document.Order","items":[{"name":" spaced "}]}),
                3,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.path(), Some("ops[3].args.items[0].name"));
        }
    }

    #[test]
    fn typed_attribute_changes_keep_the_member_owner_scope_and_caller_fields() {
        use super::{AddressedFields, MetadataPlanKind};
        use unica_metadata::{MetaCollection, MetaEditOperation};
        let fixture = MetadataFixture::new();
        for (at, section) in [
            ("Document.Order.Attribute.Total", None),
            (
                "main:Document.Order.TabularSection.Lines.Attribute.Total",
                Some("Lines"),
            ),
        ] {
            let parsed = parse_metadata_plan_operation(
                "attribute.set",
                &json!({"at":at,"values":{"newName":"Changed","comment":"kept"}}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let MetadataPlanKind::Edit {
                target,
                operation:
                    MetaEditOperation::Update {
                        collection,
                        scope,
                        elements,
                    },
                addressed,
            } = parsed.kind
            else {
                panic!("attribute.set must plan a typed update");
            };
            assert_eq!(target.as_str(), "Document.Order");
            assert_eq!(collection, MetaCollection::Attributes);
            assert_eq!(
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
                section
            );
            assert_eq!(elements.len(), 1);
            assert_eq!(elements[0].name, "Total");
            assert_eq!(elements[0].new_name.as_deref(), Some("Changed"));
            assert_eq!(elements[0].comment.as_deref(), Some("kept"));
            assert_eq!(addressed, Some(AddressedFields::Values));

            let parsed = parse_metadata_plan_operation(
                "attribute.remove",
                &json!({"at":at}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let MetadataPlanKind::Edit {
                target,
                operation:
                    MetaEditOperation::Remove {
                        collection,
                        scope,
                        names,
                    },
                addressed,
            } = parsed.kind
            else {
                panic!("attribute.remove must plan a typed removal");
            };
            assert_eq!(target.as_str(), "Document.Order");
            assert_eq!(collection, MetaCollection::Attributes);
            assert_eq!(
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
                section
            );
            assert_eq!(names, ["Total"]);
            assert_eq!(addressed, Some(AddressedFields::None));
        }
    }

    #[test]
    fn typed_attribute_changes_keep_writer_parser_refusal_order() {
        let fixture = MetadataFixture::new();
        for (operation, args, path) in [
            (
                "attribute.set",
                json!({"at":null,"values":false,"unknown":true}),
                "unknown",
            ),
            (
                "attribute.remove",
                json!({"at":null,"unknown":true}),
                "unknown",
            ),
            ("attribute.set", json!({"at":42,"values":false}), "at"),
            ("attribute.remove", json!({"at":42}), "at"),
            (
                "attribute.set",
                json!({"at":"main:Document.Order","values":false}),
                "at",
            ),
            (
                "attribute.remove",
                json!({"at":"main:Document.Order"}),
                "at",
            ),
            (
                "attribute.set",
                json!({"at":"main:Document.Order.Attribute.Total","values":false}),
                "values",
            ),
            (
                "attribute.set",
                json!({"at":"main:Document.Order.Attribute.Total","values":{"name":42,"newName":false}}),
                "values.name",
            ),
            (
                "attribute.set",
                json!({"at":"main:Document.Order.Attribute.Total","values":{"newName":false,"unknown":true}}),
                "values.unknown",
            ),
        ] {
            let error =
                parse_metadata_plan_operation(operation, &args, 3, &fixture.binding).unwrap_err();
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::BadValue,
                "{operation}: {args}"
            );
            assert_eq!(
                error.path(),
                Some(format!("ops[3].args.{path}").as_str()),
                "{operation}: {args}: {error:?}"
            );
        }
    }

    #[test]
    fn typed_command_changes_keep_the_addressed_collection_action_and_fields() {
        use super::{AddressedFields, MetadataPlanKind};
        use unica_metadata::{MetaCollection, MetaEditOperation};
        let fixture = MetadataFixture::new();
        for at in [
            "Document.Order.Command.Print",
            "main:Document.Order.Command.Print",
        ] {
            let parsed = parse_metadata_plan_operation(
                "command.set",
                &json!({"at":at,"values":{
                    "newName":"PrintCopy","synonym":"Print copy","comment":"kept",
                    "position":{"before":"Other"}
                }}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let MetadataPlanKind::Edit {
                target,
                operation:
                    MetaEditOperation::Update {
                        collection,
                        scope,
                        elements,
                    },
                addressed,
            } = parsed.kind
            else {
                panic!("command.set must plan a typed update");
            };
            assert_eq!(target.as_str(), "Document.Order");
            assert_eq!(collection, MetaCollection::Commands);
            assert!(scope.is_none());
            assert_eq!(elements.len(), 1);
            assert_eq!(elements[0].name, "Print");
            assert_eq!(elements[0].new_name.as_deref(), Some("PrintCopy"));
            assert_eq!(elements[0].synonym.as_deref(), Some("Print copy"));
            assert_eq!(elements[0].comment.as_deref(), Some("kept"));
            assert_eq!(
                elements[0].position.as_ref().unwrap().before.as_deref(),
                Some("Other")
            );
            assert_eq!(addressed, Some(AddressedFields::Values));

            let parsed = parse_metadata_plan_operation(
                "command.remove",
                &json!({"at":at}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let MetadataPlanKind::Edit {
                target,
                operation:
                    MetaEditOperation::Remove {
                        collection,
                        scope,
                        names,
                    },
                addressed,
            } = parsed.kind
            else {
                panic!("command.remove must plan a typed removal");
            };
            assert_eq!(target.as_str(), "Document.Order");
            assert_eq!(collection, MetaCollection::Commands);
            assert!(scope.is_none());
            assert_eq!(names, ["Print"]);
            assert_eq!(addressed, Some(AddressedFields::None));
        }
    }

    #[test]
    fn typed_command_changes_keep_writer_parser_refusal_order() {
        use serde_json::Map;
        let fixture = MetadataFixture::new();
        for (operation, args, path) in [
            ("command.set", json!(false), ""),
            (
                "command.set",
                json!({"at":null,"values":false,"unknown":true}),
                ".unknown",
            ),
            (
                "command.remove",
                json!({"at":null,"unknown":true}),
                ".unknown",
            ),
            (
                "command.remove",
                json!({"at":null,"values":false}),
                ".values",
            ),
            ("command.set", json!({"at":42,"values":false}), ".at"),
            ("command.remove", json!({"at":42}), ".at"),
            (
                "command.set",
                json!({"at":"main:Document.Order","values":false}),
                ".at",
            ),
            ("command.remove", json!({"at":"main:Document.Order"}), ".at"),
            (
                "command.set",
                json!({"at":"main:Document.Order.Attribute.Total","values":false}),
                ".at",
            ),
            (
                "command.set",
                json!({"at":"main:Document.Order.Form.Main.Command.Print","values":false}),
                ".at",
            ),
            (
                "command.remove",
                json!({"at":"main:Document.Order.Command"}),
                ".at",
            ),
            (
                "command.set",
                json!({"at":"main:Document.Order.Command.Print","values":false}),
                ".values",
            ),
            (
                "command.set",
                json!({"at":"main:Document.Order.Command.Print","values":{}}),
                ".values",
            ),
            (
                "command.set",
                json!({"at":"main:Document.Order.Command.Print","values":{"name":42,"newName":false}}),
                ".values.name",
            ),
            (
                "command.set",
                json!({"at":"main:Document.Order.Command.Print","values":{"newName":false,"unknown":true}}),
                ".values.unknown",
            ),
        ] {
            let error =
                parse_metadata_plan_operation(operation, &args, 3, &fixture.binding).unwrap_err();
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::BadValue,
                "{operation}: {args}"
            );
            assert_eq!(
                error.path(),
                Some(format!("ops[3].args{path}").as_str()),
                "{operation}: {args}: {error:?}"
            );
        }
        for (field, value) in [
            ("newName", json!("1Print")),
            ("title", json!("Form-only")),
            ("action", json!("Print")),
            ("type", json!({"variants":[{"kind":"boolean"}]})),
            ("required", json!(true)),
            ("fillValue", json!({"kind":"boolean","value":true})),
            ("indexing", json!("Index")),
            ("attributes", json!([{"name":"Nested"}])),
        ] {
            let values = Map::from_iter([(field.to_string(), value)]);
            let error = parse_metadata_plan_operation(
                "command.set",
                &json!({"at":"main:Document.Order.Command.Print","values":values}),
                3,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{field}");
            assert_eq!(
                error.path(),
                Some(format!("ops[3].args.values.{field}").as_str()),
                "{field}: {error:?}"
            );
        }
    }

    #[test]
    fn typed_member_changes_keep_each_addressed_collection_and_caller_fields() {
        use super::{AddressedFields, MetadataPlanKind};
        use unica_metadata::{MetaCollection, MetaEditOperation};
        let fixture = MetadataFixture::new();
        for (family, owner, leaf, collection) in [
            (
                "tabularSection",
                "Document.Order",
                "TabularSection",
                MetaCollection::TabularSections,
            ),
            (
                "dimension",
                "InformationRegister.Prices",
                "Dimension",
                MetaCollection::Dimensions,
            ),
            (
                "resource",
                "InformationRegister.Prices",
                "Resource",
                MetaCollection::Resources,
            ),
            (
                "enumValue",
                "Enum.Kinds",
                "EnumValue",
                MetaCollection::EnumValues,
            ),
            (
                "column",
                "DocumentJournal.All",
                "Column",
                MetaCollection::Columns,
            ),
        ] {
            for at in [
                format!("{owner}.{leaf}.Member"),
                format!("main:{owner}.{leaf}.Member"),
            ] {
                let operation = format!("{family}.set");
                let parsed = parse_metadata_plan_operation(
                    &operation,
                    &json!({"at":at,"values":{"newName":"Changed","comment":"kept"}}),
                    3,
                    &fixture.binding,
                )
                .unwrap();
                let MetadataPlanKind::Edit {
                    target,
                    operation:
                        MetaEditOperation::Update {
                            collection: actual,
                            scope,
                            elements,
                        },
                    addressed,
                } = parsed.kind
                else {
                    panic!("{operation} must plan a typed metadata update");
                };
                assert_eq!(target.as_str(), owner, "{operation}");
                assert_eq!(actual, collection, "{operation}");
                assert!(scope.is_none(), "{operation}");
                assert_eq!(elements.len(), 1, "{operation}");
                assert_eq!(elements[0].name, "Member", "{operation}");
                assert_eq!(
                    elements[0].new_name.as_deref(),
                    Some("Changed"),
                    "{operation}"
                );
                assert_eq!(elements[0].comment.as_deref(), Some("kept"), "{operation}");
                assert_eq!(addressed, Some(AddressedFields::Values), "{operation}");

                let operation = format!("{family}.remove");
                let parsed = parse_metadata_plan_operation(
                    &operation,
                    &json!({"at":at}),
                    3,
                    &fixture.binding,
                )
                .unwrap();
                let MetadataPlanKind::Edit {
                    target,
                    operation:
                        MetaEditOperation::Remove {
                            collection: actual,
                            scope,
                            names,
                        },
                    addressed,
                } = parsed.kind
                else {
                    panic!("{operation} must plan a typed metadata removal");
                };
                assert_eq!(target.as_str(), owner, "{operation}");
                assert_eq!(actual, collection, "{operation}");
                assert!(scope.is_none(), "{operation}");
                assert_eq!(names, ["Member"], "{operation}");
                assert_eq!(addressed, Some(AddressedFields::None), "{operation}");
            }
        }
    }

    #[test]
    fn typed_member_operations_keep_writer_parser_refusal_order() {
        let fixture = MetadataFixture::new();
        for (family, owner, leaf) in [
            ("tabularSection", "Document.Order", "TabularSection"),
            ("dimension", "InformationRegister.Prices", "Dimension"),
            ("resource", "InformationRegister.Prices", "Resource"),
            ("enumValue", "Enum.Kinds", "EnumValue"),
            ("column", "DocumentJournal.All", "Column"),
        ] {
            for verb in ["add", "set", "remove"] {
                let operation = format!("{family}.{verb}");
                let at = format!("main:{owner}.{leaf}.Member");
                let mut samples = vec![
                    (json!(false), ""),
                    (json!({"at":null,"unknown":true}), ".unknown"),
                    (json!({"at":42}), ".at"),
                ];
                if verb == "add" {
                    samples.extend([
                        (
                            json!({"at":format!("main:{owner}"),"items":false}),
                            ".items",
                        ),
                        (
                            json!({"at":format!("main:{owner}"),"items":[{"name":" spaced "}]}),
                            ".items[0].name",
                        ),
                        (json!({"at":at,"items":false}), ".at"),
                    ]);
                } else {
                    samples.extend([
                        (json!({"at":format!("main:{owner}")}), ".at"),
                        (json!({"at":format!("main:{owner}.Attribute.Other")}), ".at"),
                        (json!({"at":format!("main:{owner}.{leaf}")}), ".at"),
                    ]);
                    if verb == "set" {
                        samples.extend([
                            (json!({"at":at,"values":false}), ".values"),
                            (json!({"at":at,"values":{}}), ".values"),
                            (
                                json!({"at":at,"values":{"name":42,"newName":false}}),
                                ".values.name",
                            ),
                            (
                                json!({"at":at,"values":{"newName":false,"unknown":true}}),
                                ".values.unknown",
                            ),
                        ]);
                    } else {
                        samples.push((json!({"at":null,"values":false}), ".values"));
                    }
                }
                for (args, path) in samples {
                    let error =
                        parse_metadata_plan_operation(&operation, &args, 3, &fixture.binding)
                            .unwrap_err();
                    assert_eq!(
                        error.kind(),
                        ApplyPlanErrorKind::BadValue,
                        "{operation}: {args}"
                    );
                    assert_eq!(
                        error.path(),
                        Some(format!("ops[3].args{path}").as_str()),
                        "{operation}: {args}: {error:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn object_create_stages_the_template_files_and_registers_the_child() {
        let fixture = MetadataFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let parsed = fixture.parse(
            "object.create",
            json!({"at": "main:Configuration", "values": {"kind": "Catalog", "name": "Товары"}}),
            0,
        );
        let (staged, effects) = plan_metadata_batch_owned_for_test(staged, authority, &[parsed])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 1);
        let changes = staged.planned_changes();
        let descriptor = changes
            .iter()
            .find(|change| change.relative_path == Path::new("Catalogs/Товары.xml"))
            .expect("the new descriptor is staged");
        assert_eq!(descriptor.kind, StagedChangeKind::Create);
        let owner = changes
            .iter()
            .find(|change| change.relative_path == Path::new("Configuration.xml"))
            .expect("the owner registration is staged");
        assert_eq!(owner.kind, StagedChangeKind::Replace);
        let StagedFileState::Bytes(owner_bytes) = &owner.current else {
            panic!("owner keeps bytes");
        };
        let owner_text = String::from_utf8(owner_bytes.clone()).unwrap();
        assert!(
            owner_text.contains("<Catalog>Товары</Catalog>"),
            "{owner_text}"
        );
        assert!(
            owner_text.contains("<Document>Order</Document>"),
            "{owner_text}"
        );
    }

    /// #1332: registers of one batch share the registrar document. Each
    /// operation adds its record to the image the previous one staged
    /// instead of refusing because the disk preimage no longer matches.
    #[test]
    fn object_create_batches_registers_that_share_one_registrar_document() {
        let order = ORDER_XML.replace(
            "<BasedOn/></Properties>",
            "<BasedOn/><RegisterRecords/></Properties>",
        );
        let configuration = RU_CONFIGURATION_XML.replace(
            "<Document>Order</Document>",
            "<Document>Order</Document><ChartOfCalculationTypes>Kinds</ChartOfCalculationTypes>",
        );
        let chart = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ChartOfCalculationTypes uuid="44444444-4444-4444-8444-444444444444"><Properties><Name>Kinds</Name></Properties><ChildObjects/></ChartOfCalculationTypes></MetaDataObject>"#;
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", &configuration),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", &order),
            ("ChartsOfCalculationTypes/Kinds.xml", chart),
        ]);
        let registers = [
            ("AccumulationRegister", "Продажи"),
            ("AccumulationRegister", "Остатки"),
            ("AccumulationRegister", "Обороты"),
            ("CalculationRegister", "Начисления"),
            ("CalculationRegister", "Премии"),
        ];
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = registers
            .iter()
            .enumerate()
            .map(|(index, (kind, name))| {
                fixture.parse(
                    "object.create",
                    json!({"at": "main:Configuration", "values": {"kind": kind, "name": name}}),
                    index,
                )
            })
            .collect::<Vec<_>>();

        let (staged, effects) = plan_metadata_batch_owned_for_test(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));

        assert_eq!(effects.len(), registers.len());
        let changes = staged.planned_changes();
        let document = changes
            .iter()
            .find(|change| change.relative_path == Path::new("Documents/Order.xml"))
            .expect("the registrar document is staged once");
        assert_eq!(
            document.original,
            StagedFileState::Bytes(order.clone().into_bytes())
        );
        let StagedFileState::Bytes(bytes) = &document.current else {
            panic!("registrar keeps bytes");
        };
        let records = registers
            .iter()
            .map(|(kind, name)| {
                format!("<xr:Item xsi:type=\"xr:MDObjectRef\">{kind}.{name}</xr:Item>")
            })
            .collect::<String>();
        assert_eq!(
            String::from_utf8(bytes.clone()).unwrap(),
            order.replace(
                "<RegisterRecords/>",
                &format!("<RegisterRecords>{records}</RegisterRecords>")
            )
        );
        for (kind, name) in registers {
            let directory = if kind == "AccumulationRegister" {
                "AccumulationRegisters"
            } else {
                "CalculationRegisters"
            };
            let descriptor = PathBuf::from(directory).join(format!("{name}.xml"));
            assert!(
                changes
                    .iter()
                    .any(|change| change.relative_path == descriptor
                        && change.kind == StagedChangeKind::Create),
                "{descriptor:?} is not staged"
            );
        }
        assert_eq!(fixture.disk_bytes(), order.as_bytes());
    }

    /// A journal guards the registrar document that a register of the same
    /// batch has already changed; the order of the two must not matter.
    #[test]
    fn object_create_guards_a_registrar_changed_earlier_in_the_batch() {
        let order = ORDER_XML.replace(
            "<BasedOn/></Properties>",
            "<BasedOn/><RegisterRecords/></Properties>",
        );
        for objects in [
            [
                ("AccumulationRegister", "Продажи"),
                ("DocumentJournal", "Журнал"),
            ],
            [
                ("DocumentJournal", "Журнал"),
                ("AccumulationRegister", "Продажи"),
            ],
        ] {
            let fixture = MetadataFixture::with_files(&[
                ("Configuration.xml", RU_CONFIGURATION_XML),
                ("Languages/Русский.xml", &language_xml("Русский", "ru")),
                ("Documents/Order.xml", &order),
            ]);
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let operations = objects
                .iter()
                .enumerate()
                .map(|(index, (kind, name))| {
                    fixture.parse(
                        "object.create",
                        json!({"at": "main:Configuration", "values": {"kind": kind, "name": name}}),
                        index,
                    )
                })
                .collect::<Vec<_>>();

            let (staged, effects) =
                plan_metadata_batch_owned_for_test(staged, authority, &operations)
                    .unwrap_or_else(|error| panic!("{objects:?}: {error:?} at {:?}", error.path()));

            assert_eq!(effects.len(), 2, "{objects:?}");
            let changes = staged.planned_changes();
            for descriptor in [
                "AccumulationRegisters/Продажи.xml",
                "DocumentJournals/Журнал.xml",
            ] {
                assert!(
                    changes
                        .iter()
                        .any(|change| change.relative_path == Path::new(descriptor)),
                    "{objects:?}: {descriptor} is not staged"
                );
            }
            let document = changes
                .iter()
                .find(|change| change.relative_path == Path::new("Documents/Order.xml"))
                .expect("the registrar document is staged");
            let StagedFileState::Bytes(bytes) = &document.current else {
                panic!("registrar keeps bytes");
            };
            assert!(
                String::from_utf8_lossy(bytes).contains("AccumulationRegister.Продажи"),
                "{objects:?}"
            );
            assert_eq!(fixture.disk_bytes(), order.as_bytes());
        }
    }

    /// #1385: on a platform-shaped CRLF document the new header attribute
    /// lands before the tabular sections and forms, the new template before
    /// the commands, and every inserted line keeps the descriptor's CRLF.
    #[test]
    fn member_and_template_adds_take_their_platform_slot_and_line_ending() {
        let order = ORDER_XML
            .replace(
                "\t\t<ChildObjects/>\n",
                concat!(
                    "\t\t<ChildObjects>\n",
                    "\t\t\t<Attribute uuid=\"22222222-2222-4222-8222-222222222221\">\n",
                    "\t\t\t\t<Properties><Name>Number2</Name><Synonym/><Comment/></Properties>\n",
                    "\t\t\t</Attribute>\n",
                    "\t\t\t<Form>Main</Form>\n",
                    "\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n",
                    "\t\t\t\t<Properties><Name>Rows</Name><Synonym/><Comment/></Properties>\n",
                    "\t\t\t\t<ChildObjects/>\n",
                    "\t\t\t</TabularSection>\n",
                    "\t\t\t<Command uuid=\"22222222-2222-4222-8222-222222222223\">\n",
                    "\t\t\t\t<Properties><Name>Open</Name><Synonym/><Comment/></Properties>\n",
                    "\t\t\t</Command>\n",
                    "\t\t</ChildObjects>\n",
                ),
            )
            .replace('\n', "\r\n");
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", RU_CONFIGURATION_XML),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", &order),
        ]);
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let attribute = fixture.parse(
            "attribute.add",
            json!({
                "at": "main:Document.Order",
                "items": [{
                    "name": "Comment2",
                    "type": {"variants": [{"kind": "string", "length": 25, "allowedLength": "variable"}]}
                }]
            }),
            0,
        );
        let template = fixture.parse(
            "template.add",
            json!({
                "at": "main:Document.Order",
                "items": [{"name": "Print", "templateType": "SpreadsheetDocument"}]
            }),
            1,
        );
        let (staged, _) =
            plan_metadata_batch_owned_for_test(staged, authority, &[attribute, template])
                .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let descriptor = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path == Path::new("Documents/Order.xml"))
            .expect("the owner descriptor is staged");
        let StagedFileState::Bytes(bytes) = descriptor.current else {
            panic!("descriptor keeps bytes");
        };
        let text = String::from_utf8(bytes).unwrap();
        let bare_lf = text
            .match_indices('\n')
            .filter(|(index, _)| *index == 0 || text.as_bytes()[index - 1] != b'\r')
            .count();
        assert_eq!(bare_lf, 0, "inserted lines must keep the CRLF line ending");
        let document = roxmltree::Document::parse(&text).unwrap();
        let children = document
            .descendants()
            .find(|node| node.has_tag_name("ChildObjects"))
            .unwrap()
            .children()
            .filter(|node| node.is_element())
            .map(|node| {
                let name = node
                    .descendants()
                    .find(|inner| inner.has_tag_name("Name"))
                    .and_then(|inner| inner.text())
                    .or_else(|| node.text())
                    .unwrap_or_default();
                format!("{}:{name}", node.tag_name().name())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            children,
            [
                "Attribute:Number2",
                "Attribute:Comment2",
                "Form:Main",
                "TabularSection:Rows",
                "Template:Print",
                "Command:Open",
            ]
        );
    }

    /// A descriptor with mixed line endings is not normalized by a member
    /// add: no line ending is chosen for its author (INV.SOURCE.OBSERVED-BYTES).
    #[test]
    fn member_add_leaves_a_mixed_line_ending_descriptor_unnormalized() {
        let order = ORDER_XML.replace('\n', "\r\n").replacen(
            "<Name>Order</Name>",
            "<Name>Order</Name>\n",
            1,
        );
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", RU_CONFIGURATION_XML),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", &order),
        ]);
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let attribute = fixture.parse(
            "attribute.add",
            json!({"at": "main:Document.Order", "items": [{"name": "Comment2"}]}),
            0,
        );
        let (staged, _) = plan_metadata_batch_owned_for_test(staged, authority, &[attribute])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let descriptor = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path == Path::new("Documents/Order.xml"))
            .expect("the owner descriptor is staged");
        let StagedFileState::Bytes(bytes) = descriptor.current else {
            panic!("descriptor keeps bytes");
        };
        let text = String::from_utf8(bytes).unwrap();
        let untouched = order.split("<ChildObjects/>").next().unwrap();
        assert!(
            text.starts_with(untouched),
            "the lines before the change keep their bytes"
        );
        assert!(text.contains("<Name>Comment2</Name>"));
    }

    #[test]
    fn object_create_then_attribute_add_compose_in_one_batch() {
        let fixture = MetadataFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let create = fixture.parse(
            "object.create",
            json!({"at": "main:Configuration", "values": {"kind": "Catalog", "name": "Товары"}}),
            0,
        );
        let add = fixture.parse(
            "attribute.add",
            json!({
                "at": "main:Catalog.Товары",
                "items": [{
                    "name": "Артикул",
                    "type": {"variants": [{"kind": "string", "length": 25, "allowedLength": "variable"}]}
                }]
            }),
            1,
        );
        let (staged, effects) =
            plan_metadata_batch_owned_for_test(staged, authority, &[create, add])
                .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 2);
        let descriptor = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path == Path::new("Catalogs/Товары.xml"))
            .expect("the new descriptor is staged");
        let StagedFileState::Bytes(bytes) = descriptor.current else {
            panic!("descriptor keeps bytes");
        };
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("<Name>Артикул</Name>"), "{text}");
    }

    #[test]
    fn object_create_refuses_an_existing_object_and_a_non_root_target() {
        let fixture = MetadataFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let parsed = fixture.parse(
            "object.create",
            json!({"at": "main:Configuration", "values": {"kind": "Document", "name": "Order"}}),
            0,
        );
        let error = plan_metadata_batch_owned_for_test(staged, authority, &[parsed]).unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        assert_eq!(error.path(), Some("ops[0].args.values.name"));

        let misplaced = parse_metadata_plan_operation(
            "object.create",
            &json!({"at": "main:Document.Order", "values": {"kind": "Catalog", "name": "X"}}),
            1,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(misplaced.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(misplaced.path(), Some("ops[1].args.at"));
    }

    #[test]
    fn object_remove_stages_descriptor_removal_and_owner_deregistration() {
        let fixture = MetadataFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let parsed = fixture.parse("object.remove", json!({"at": "main:Document.Order"}), 0);
        let (staged, effects) = plan_metadata_batch_owned_for_test(staged, authority, &[parsed])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 1);
        let changes = staged.planned_changes();
        let descriptor = changes
            .iter()
            .find(|change| change.relative_path == Path::new("Documents/Order.xml"))
            .expect("the descriptor removal is staged");
        assert_eq!(descriptor.kind, StagedChangeKind::Remove);
        let owner = changes
            .iter()
            .find(|change| change.relative_path == Path::new("Configuration.xml"))
            .expect("the owner deregistration is staged");
        let StagedFileState::Bytes(owner_bytes) = &owner.current else {
            panic!("owner keeps bytes");
        };
        let owner_text = String::from_utf8(owner_bytes.clone()).unwrap();
        assert!(
            !owner_text.contains("<Document>Order</Document>"),
            "{owner_text}"
        );
    }

    #[test]
    fn object_remove_retains_unchanged_reference_and_subsystem_inputs() {
        for subsystem in [false, true] {
            let fixture = MetadataFixture::new();
            let root = fixture.descriptor.parent().unwrap().parent().unwrap();
            let input = if subsystem {
                root.join("Subsystems/Main.xml")
            } else {
                root.join("CommonModules/Helper/Ext/Module.bsl")
            };
            std::fs::create_dir_all(input.parent().unwrap()).unwrap();
            let original = if subsystem {
                b"<Subsystem><Content/></Subsystem>".as_slice()
            } else {
                b"Procedure Helper()\nEndProcedure".as_slice()
            };
            std::fs::write(&input, original).unwrap();
            let admission = fixture.admission();
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let operation = fixture.parse("object.remove", json!({"at": "main:Document.Order"}), 0);
            let (staged, _) = plan_metadata_batch_owned_for_test(
                admission.staged_state().unwrap(),
                authority,
                &[operation],
            )
            .unwrap();
            let prepared = admission.prepare(staged).unwrap();
            let changed = if subsystem {
                b"<Subsystem><Content><Item>Document.Order</Item></Content></Subsystem>".as_slice()
            } else {
                b"Procedure Helper()\n x = Documents.Order;\nEndProcedure".as_slice()
            };
            std::fs::write(&input, changed).unwrap();
            assert!(fixture.actor.publish_prepared_apply(prepared).is_err());
            assert_eq!(fixture.disk_bytes(), ORDER_XML.as_bytes());
            assert_eq!(std::fs::read(&input).unwrap(), changed);
        }
    }

    #[test]
    fn help_create_stages_the_embedded_help_facet() {
        let fixture = MetadataFixture::new();
        std::fs::create_dir_all(fixture.descriptor.with_extension("").join("Ext")).unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let parsed = fixture.parse(
            "help.create",
            json!({"at": "main:Document.Order", "values": {"lang": "ru"}}),
            0,
        );
        let (staged, effects) = plan_metadata_batch_owned_for_test(staged, authority, &[parsed])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 1);
        let mut created = staged
            .planned_changes()
            .into_iter()
            .filter(|change| change.kind == StagedChangeKind::Create)
            .map(|change| change.relative_path)
            .collect::<Vec<_>>();
        created.sort();
        // Component order: the `Help` directory sorts before `Help.xml`.
        assert_eq!(
            created,
            vec![
                PathBuf::from("Documents/Order/Ext/Help/ru.html"),
                PathBuf::from("Documents/Order/Ext/Help.xml"),
            ]
        );
    }

    #[test]
    fn member_collections_relations_and_help_plan_through_the_typed_engine() {
        let fixture = MetadataFixture::new();
        let cases = [
            (
                "tabularSection.add",
                json!({"at": "main:Document.Order", "items": [{
                    "name": "Строки",
                    "attributes": [{
                        "name": "Сумма",
                        "type": {"variants": [{"kind": "number", "digits": 10, "fraction": 2, "sign": "any"}]}
                    }]
                }]}),
                "<TabularSection",
            ),
            (
                "command.add",
                json!({"at": "main:Document.Order", "items": [{"name": "ПечатьАкта"}]}),
                "<Command",
            ),
            (
                "template.add",
                json!({"at": "main:Document.Order", "items": [{
                    "name": "ПФ_MXL_Акт",
                    "templateType": "SpreadsheetDocument"
                }]}),
                "<Template",
            ),
            (
                "relation.add",
                json!({"at": "main:Document.Order", "values": {
                    "relation": "basedOn",
                    "targets": [{"metadataPath": "Document.Order"}]
                }}),
                "<BasedOn",
            ),
        ];
        for (name, args, marker) in cases {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let parsed = fixture.parse(name, args, 0);
            let (staged, effects) =
                plan_metadata_batch_owned_for_test(staged, authority, &[parsed])
                    .unwrap_or_else(|error| panic!("{name}: {error:?} at {:?}", error.path()));
            assert_eq!(effects.len(), 1, "{name}");
            let changed = staged
                .planned_changes()
                .iter()
                .map(|change| match &change.current {
                    StagedFileState::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
                    StagedFileState::Absent => String::new(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                changed.contains(marker),
                "{name}: staged postimage misses {marker}"
            );
        }
    }

    /// Табличная часть адресуется (#1352): реквизит, добавленный на адресе
    /// новой табличной части в том же пакете, попадает внутрь неё, а не в
    /// шапку; соседние реквизиты и сгенерированные типы части остаются.
    /// Переименование члена — `newName` на его адресе.
    #[test]
    fn a_tabular_section_takes_attributes_at_its_own_address_in_one_batch() {
        let fixture = MetadataFixture::new();
        let staged = plan_all(
            &fixture,
            &[
                (
                    "attribute.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Total"}]}),
                ),
                (
                    "tabularSection.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Lines", "attributes": [{"name": "Qty"}]}]}),
                ),
                (
                    "attribute.add",
                    json!({"at": "main:Document.Order.TabularSection.Lines", "items": [{"name": "Price"}]}),
                ),
                (
                    "tabularSection.set",
                    json!({"at": "main:Document.Order.TabularSection.Lines", "values": {"synonym": "Строки"}}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        let order = &texts[Path::new("Documents/Order.xml")];
        let section_start = order.find("<TabularSection").expect("the section is added");
        let section_end = order.find("</TabularSection>").expect("the section closes");
        let section = &order[section_start..section_end];
        for attribute in ["<Name>Qty</Name>", "<Name>Price</Name>"] {
            assert!(
                section.contains(attribute),
                "{attribute} in the section: {order}"
            );
        }
        // Без позиции новый реквизит встаёт после существующих, как в шапке.
        assert!(
            section.find("<Name>Qty</Name>") < section.find("<Name>Price</Name>"),
            "{section}"
        );
        let header = format!("{}{}", &order[..section_start], &order[section_end..]);
        assert!(header.contains("<Name>Total</Name>"), "{order}");
        assert!(
            !header.contains("<Name>Price</Name>"),
            "the section attribute is not a header attribute: {order}"
        );
        // Сгенерированные типы новой части по одному и не размножены
        // последующими правками.
        for generated in [
            "DocumentTabularSection.Order.Lines",
            "DocumentTabularSectionRow.Order.Lines",
        ] {
            assert_eq!(
                order.matches(&format!("name=\"{generated}\"")).count(),
                1,
                "{generated}: {order}"
            );
        }
        assert!(section.contains("Строки"), "{section}");
    }

    /// Член называется адресом: `values.name` и адрес владельца — ошибки
    /// вызова с точным путём; удаление не принимает аргументов.
    #[test]
    fn addressed_member_operations_refuse_a_name_argument_and_the_owner_address() {
        let fixture = MetadataFixture::new();
        for (operation, args, path) in [
            (
                "tabularSection.set",
                json!({"at": "main:Document.Order.TabularSection.Lines", "values": {"name": "Lines", "synonym": "x"}}),
                "ops[0].args.values.name",
            ),
            (
                "tabularSection.set",
                json!({"at": "main:Document.Order", "values": {"synonym": "x"}}),
                "ops[0].args.at",
            ),
            (
                "tabularSection.set",
                json!({"at": "main:Document.Order.TabularSection.Lines", "values": {}}),
                "ops[0].args.values",
            ),
            (
                "tabularSection.remove",
                json!({"at": "main:Document.Order.TabularSection.Lines", "values": {"name": "Lines"}}),
                "ops[0].args.values",
            ),
            (
                "dimension.set",
                json!({"at": "main:Document.Order.TabularSection.Lines", "values": {"synonym": "x"}}),
                "ops[0].args.at",
            ),
            (
                "command.remove",
                json!({"at": "main:Document.Order.Command"}),
                "ops[0].args.at",
            ),
            (
                "template.set",
                json!({"at": "main:Document.Order", "values": {"name": "Print", "synonym": "x"}}),
                "ops[0].args.at",
            ),
            (
                "template.set",
                json!({"at": "main:Document.Order.Template.Print", "values": {"name": "Print"}}),
                "ops[0].args.values.name",
            ),
            (
                "template.remove",
                json!({"at": "main:Document.Order.Template.Print", "values": {"name": "Print"}}),
                "ops[0].args.values",
            ),
            (
                "attribute.add",
                json!({"at": "main:Document.Order", "scope": {"tabularSection": "Lines"}, "items": [{"name": "Price"}]}),
                "ops[0].args.scope",
            ),
            (
                "attribute.add",
                json!({"at": "main:Document.Order.TabularSection.Lines.Attribute.Qty", "items": [{"name": "Price"}]}),
                "ops[0].args.at",
            ),
            (
                "predefinedItem.set",
                json!({"at": "main:Document.Order.PredefinedItem.Main", "values": {"id": "a1b2c3d4-1111-4111-8111-111111111111", "description": "x"}}),
                "ops[0].args.values.id",
            ),
            (
                "predefinedItem.set",
                json!({"at": "main:Document.Order.PredefinedItem.Main", "values": {"name": "Other"}}),
                "ops[0].args.values.name",
            ),
            (
                "predefinedItem.remove",
                json!({"at": "main:Document.Order", "values": {"id": "a1b2c3d4-1111-4111-8111-111111111111"}}),
                "ops[0].args.values",
            ),
        ] {
            let error = parse_metadata_plan_operation(operation, &args, 0, &fixture.binding)
                .expect_err(operation);
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::BadValue,
                "{operation} {args}"
            );
            assert_eq!(error.path(), Some(path), "{operation} {args}: {error:?}");
        }
    }

    /// Изменение отсутствующего члена — «не найден» по адресу операции:
    /// `next` отказа ведёт к владельцу, где член создают.
    #[test]
    fn a_missing_addressed_member_is_not_found_at_the_operation_address() {
        let fixture = MetadataFixture::new();
        for (operation, args) in [
            (
                "tabularSection.set",
                json!({"at": "main:Document.Order.TabularSection.Absent", "values": {"synonym": "x"}}),
            ),
            (
                "tabularSection.remove",
                json!({"at": "main:Document.Order.TabularSection.Absent"}),
            ),
            (
                "attribute.set",
                json!({"at": "main:Document.Order.Attribute.Absent", "values": {"synonym": "x"}}),
            ),
            (
                "attribute.add",
                json!({"at": "main:Document.Order.TabularSection.Absent", "items": [{"name": "Price"}]}),
            ),
        ] {
            let error = plan_all(&fixture, &[(operation, args)]).expect_err(operation);
            assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound, "{operation}");
            assert_eq!(error.path(), Some("ops[0].args.at"), "{operation}");
        }
        // Отсутствует не сам член, а названный в аргументах сосед: отказ
        // указывает на аргумент, а не на адрес.
        let error = plan_all(
            &fixture,
            &[
                (
                    "tabularSection.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Lines"}]}),
                ),
                (
                    "tabularSection.set",
                    json!({"at": "main:Document.Order.TabularSection.Lines", "values": {"position": {"before": "Absent"}}}),
                ),
            ],
        )
        .expect_err("a missing anchor refuses");
        assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound);
        assert_eq!(error.path(), Some("ops[1].args.values.position"));
    }

    /// Опубликованный контракт операции члена и разбор планировщиком
    /// согласны: структурно неверный вызов отвергают оба, верный принимают
    /// оба. Контракт берётся на конкретном узле, как его отдаёт `view`
    /// адресом `<узел>.Can.<op>`: поля, зависящие от вида владельца и табличной части,
    /// сужены до узла. Каждое семейство проверяется на нескольких видах
    /// владельца. Перекрёстные правила полей (значение заполнения требует
    /// тип) схема не выражает — такие образцы здесь не берутся.
    #[test]
    fn member_operation_contracts_and_the_planner_agree_on_structure() {
        use unica_address::address::QualifiedAddress;
        use unica_bsl::profile::PlatformProfile;
        use unica_metadata::{metadata_collection_spec, MetaCollection};
        let fixture = MetadataFixture::new();
        let traits = crate::domain::apply::source_set_traits_for_read(
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
        );
        let string_type =
            json!({"variants": [{"kind": "string", "length": 10, "allowedLength": "variable"}]});
        let string_fill = json!({"kind": "string", "value": "x"});
        let id = "a1b2c3d4-1111-4111-8111-111111111111";
        let registry = crate::domain::apply::operation_registry();
        let mut checked = 0;
        // Образцы, чья допустимость зависит от узла: обе стороны обязаны
        // согласиться, и по матрице встречаются оба исхода.
        let mut site_dependent = std::collections::BTreeSet::new();
        for descriptor in registry.descriptors() {
            let op = descriptor.name();
            // Реквизит и команда формы носят то же имя (#1465): здесь сверяются
            // члены метаданных.
            if op == "template.add"
                || descriptor.argument_contract().is_none()
                || descriptor.family() != unica_operations::OperationFamily::Metadata
            {
                continue;
            }
            let (family, verb) = op.split_once('.').unwrap();
            let owners: &[&str] = match family {
                "attribute" => &[
                    "main:Document.Order",
                    "main:Catalog.Items",
                    "main:Report.Sales",
                ],
                "tabularSection" | "command" => &["main:Document.Order", "main:Catalog.Items"],
                "dimension" | "resource" => &[
                    "main:InformationRegister.Prices",
                    "main:AccumulationRegister.Stock",
                ],
                "enumValue" => &["main:Enum.Kinds"],
                "column" => &["main:DocumentJournal.All"],
                "template" => &["main:Document.Order", "main:Report.Sales"],
                "predefinedItem" => &[
                    "main:Catalog.Items",
                    "main:ChartOfAccounts.Main",
                    "main:ChartOfCharacteristicTypes.Props",
                    "main:ChartOfCalculationTypes.Calc",
                ],
                _ => continue,
            };
            let leaf = match family {
                "attribute" => "Attribute",
                "tabularSection" => "TabularSection",
                "dimension" => "Dimension",
                "resource" => "Resource",
                "enumValue" => "EnumValue",
                "column" => "Column",
                "command" => "Command",
                "template" => "Template",
                _ => "PredefinedItem",
            };
            let collection = match family {
                "attribute" => Some(MetaCollection::Attributes),
                "tabularSection" => Some(MetaCollection::TabularSections),
                "dimension" => Some(MetaCollection::Dimensions),
                "resource" => Some(MetaCollection::Resources),
                "enumValue" => Some(MetaCollection::EnumValues),
                "column" => Some(MetaCollection::Columns),
                "command" => Some(MetaCollection::Commands),
                _ => None,
            };
            let spec = collection.map(metadata_collection_spec);
            let typed = spec.is_some_and(|spec| spec.allows_type);
            // `Some(verdict)` — исход задан; `None` — зависит от узла.
            let mut samples: Vec<(serde_json::Value, Option<bool>)> = Vec::new();
            match (family, verb) {
                ("predefinedItem", "add") => samples.extend([
                    (json!({"items": [{"id": id, "name": "Main"}]}), Some(true)),
                    (json!({"items": [{"id": id, "name": "Main", "code": "1"}]}), Some(true)),
                    (json!({"items": [{"name": "Main"}]}), Some(false)),
                    (json!({"items": [{"id": "not-a-uuid", "name": "Main"}]}), Some(false)),
                    (json!({"items": [{"id": id, "name": "1Main"}]}), Some(false)),
                    (json!({"items": [{"id": id, "name": "Main", "unknown": 1}]}), Some(false)),
                    (json!({"items": []}), Some(false)),
                    (json!({}), Some(false)),
                ]),
                ("predefinedItem", "set") => samples.extend([
                    (json!({"values": {"description": "x"}}), Some(true)),
                    (json!({"values": {"newName": "Other"}}), Some(true)),
                    (json!({"values": {"newName": "1Other"}}), Some(false)),
                    (json!({"values": {"newName": ""}}), Some(false)),
                    (json!({"values": {"id": id}}), Some(false)),
                    (json!({"values": {"name": "Other"}}), Some(false)),
                    (json!({"values": {}}), Some(false)),
                    (json!({}), Some(false)),
                ]),
                ("template", "set") => samples.extend([
                    (json!({"values": {"comment": "x"}}), Some(true)),
                    (json!({"values": {"name": "Print"}}), Some(false)),
                    (json!({"values": {}}), Some(false)),
                    (json!({"values": {"templateType": "Picture"}}), Some(false)),
                    (json!({"values": {"synonym": 1}}), Some(false)),
                ]),
                (_, "add") => samples.extend([
                    (json!({"items": [{"name": "Member", "comment": "x"}]}), Some(true)),
                    (json!({"items": [{"name": "Member", "type": string_type}]}), Some(typed)),
                    (
                        json!({"items": [{"name": "Member", "indexing": "Index"}]}),
                        // Индексируются только типизированные поля.
                        Some(matches!(family, "attribute" | "dimension" | "resource" | "column")),
                    ),
                    (
                        json!({"items": [{"name": "Member", "required": true}]}),
                        Some(spec.is_some_and(|spec| spec.allows_required)),
                    ),
                    (
                        json!({"items": [{"name": "Member", "type": string_type, "fillValue": string_fill}]}),
                        None,
                    ),
                    (
                        json!({"items": [{"name": "Member", "templateType": "SpreadsheetDocument"}]}),
                        Some(false),
                    ),
                    (json!({"items": [{"name": "1Member"}]}), Some(false)),
                    (json!({"items": [{"name": "Член-1"}]}), Some(false)),
                    (json!({"items": [{"comment": "x"}]}), Some(false)),
                    (json!({"items": [{"name": "Member", "unknown": 1}]}), Some(false)),
                    (json!({"items": [{"name": "Member", "synonym": 1}]}), Some(false)),
                    (json!({"items": []}), Some(false)),
                    (json!({"items": {}}), Some(false)),
                    (json!({"values": {}}), Some(false)),
                    (json!({}), Some(false)),
                ]),
                (_, "set") => samples.extend([
                    (json!({"values": {"newName": "Other"}}), Some(true)),
                    (json!({"values": {"comment": "x", "synonym": "y"}}), Some(true)),
                    (json!({"values": {"type": string_type}}), Some(typed)),
                    (
                        json!({"values": {"required": true}}),
                        Some(spec.is_some_and(|spec| spec.allows_required)),
                    ),
                    (
                        json!({"values": {"type": string_type, "fillValue": string_fill}}),
                        None,
                    ),
                    (json!({"values": {"name": "Member"}}), Some(false)),
                    (json!({"values": {"newName": "1Other"}}), Some(false)),
                    (json!({"values": {"synonym": 1}}), Some(false)),
                    (json!({"values": {}}), Some(false)),
                    (json!({"values": "x"}), Some(false)),
                    (json!({}), Some(false)),
                ]),
                (_, "remove") => samples.extend([
                    (json!({}), Some(true)),
                    (json!({"values": {"name": "Member"}}), Some(false)),
                    (json!({"items": []}), Some(false)),
                ]),
                _ => {}
            }
            if family == "predefinedItem" && verb != "remove" {
                let container = if verb == "add" { "items" } else { "values" };
                for (field, value) in [
                    ("isFolder", json!(false)),
                    ("type", string_type.clone()),
                    ("accountType", json!("Active")),
                    ("offBalance", json!(false)),
                    ("order", json!("1")),
                    ("accountingFlags", json!({})),
                    ("extDimensionTypes", json!([])),
                    ("actionPeriodIsBase", json!(true)),
                ] {
                    let mut item = if verb == "add" {
                        json!({"id": id, "name": "Main"})
                    } else {
                        json!({})
                    };
                    item[field] = value;
                    let args = if verb == "add" {
                        json!({container: [item]})
                    } else {
                        json!({container: item})
                    };
                    samples.push((args, None));
                }
            }
            for owner in owners {
                let mut targets = vec![if verb == "add" {
                    owner.to_string()
                } else {
                    format!("{owner}.{leaf}.Member")
                }];
                if family == "attribute" {
                    targets.push(if verb == "add" {
                        format!("{owner}.TabularSection.Lines")
                    } else {
                        format!("{owner}.TabularSection.Lines.Attribute.Member")
                    });
                }
                for target in &targets {
                    let address = QualifiedAddress::parse(target).unwrap();
                    let site = crate::domain::apply::operation_site(&address, traits).unwrap();
                    assert!(descriptor.applies_at(&site), "{op} at {target}");
                    let contract = descriptor
                        .argument_contract_at(&site)
                        .unwrap_or_else(|| panic!("{op} at {target} has a contract"));
                    let validator = jsonschema::validator_for(&contract.schema)
                        .unwrap_or_else(|error| panic!("{op}: {error}"));
                    let mut all = vec![(contract.example_args.clone(), Some(true))];
                    all.extend(samples.iter().cloned());
                    for (index, (args, expected)) in all.iter().enumerate() {
                        let mut args = args.clone();
                        args["at"] = json!(target);
                        let schema = validator.is_valid(&args);
                        let planner = parse_metadata_plan_operation(op, &args, 0, &fixture.binding)
                            .map(|_| ());
                        assert_eq!(
                            schema,
                            planner.is_ok(),
                            "{op} at {target} with {args}: schema={schema}, planner={planner:?}"
                        );
                        match expected {
                            Some(expected) => assert_eq!(
                                schema, *expected,
                                "{op} at {target} with {args}: planner={planner:?}"
                            ),
                            None => {
                                site_dependent.insert((op, index, schema));
                            }
                        }
                    }
                    checked += 1;
                }
            }
        }
        // Члены: 7 семейств × 3 операции по своим владельцам, табличная
        // часть у реквизита; макет и предопределённый элемент.
        assert_eq!(checked, 64);
        for op in [
            "attribute.add",
            "attribute.set",
            "predefinedItem.add",
            "predefinedItem.set",
        ] {
            for verdict in [true, false] {
                assert!(
                    site_dependent
                        .iter()
                        .any(|(name, _, schema)| *name == op && *schema == verdict),
                    "{op}: the node-dependent samples never answered {verdict}"
                );
            }
        }
    }

    /// Предопределённые элементы пишутся в `Ext/Predefined.xml` владельца:
    /// добавление создаёт файл, изменение и удаление находят элемент по
    /// имени из адреса, в том числе добавленный ранее в том же пакете.
    #[test]
    fn predefined_items_are_staged_in_the_owner_predefined_file_by_name() {
        const ITEMS_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Catalog uuid="99999999-9999-4999-8999-999999999999">
		<Properties><Name>Items</Name><CodeLength>9</CodeLength><CodeType>String</CodeType></Properties>
		<ChildObjects/>
	</Catalog>
</MetaDataObject>
"#;
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", RU_CONFIGURATION_XML),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", ORDER_XML),
            ("Catalogs/Items.xml", ITEMS_XML),
        ]);
        let staged = plan_all(
            &fixture,
            &[
                (
                    "predefinedItem.add",
                    json!({"at": "main:Catalog.Items", "items": [
                        {"id": "a1b2c3d4-1111-4111-8111-111111111111", "name": "Main"},
                        {"id": "a1b2c3d4-2222-4222-8222-222222222222", "name": "Spare"}
                    ]}),
                ),
                (
                    "predefinedItem.set",
                    json!({"at": "main:Catalog.Items.PredefinedItem.Main", "values": {"description": "Основной", "newName": "Primary"}}),
                ),
                (
                    "predefinedItem.remove",
                    json!({"at": "main:Catalog.Items.PredefinedItem.Spare"}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        let predefined = &texts[Path::new("Catalogs/Items/Ext/Predefined.xml")];
        assert!(predefined.contains("<Name>Primary</Name>"), "{predefined}");
        assert!(
            predefined.contains("<Description>Основной</Description>"),
            "{predefined}"
        );
        assert!(!predefined.contains("Spare"), "{predefined}");
        let missing = plan_all(
            &fixture,
            &[(
                "predefinedItem.remove",
                json!({"at": "main:Catalog.Items.PredefinedItem.Absent"}),
            )],
        )
        .expect_err("an absent item is not found");
        assert_eq!(missing.kind(), ApplyPlanErrorKind::NotFound);
        assert_eq!(missing.path(), Some("ops[0].args.at"));
        // Имя — адрес элемента: занятое имя не принимается ни при
        // добавлении (на любой глубине и без учёта регистра), ни при
        // переименовании.
        for (operations, path) in [
            (
                vec![(
                    "predefinedItem.add",
                    json!({"at": "main:Catalog.Items", "items": [
                        {"id": "a1b2c3d4-1111-4111-8111-111111111111", "name": "Дубль"},
                        {"id": "a1b2c3d4-2222-4222-8222-222222222222", "name": "дубль"}
                    ]}),
                )],
                "ops[0].args.items[1].name",
            ),
            (
                vec![
                    (
                        "predefinedItem.add",
                        json!({"at": "main:Catalog.Items", "items": [
                            {"id": "a1b2c3d4-1111-4111-8111-111111111111", "name": "Один"},
                            {"id": "a1b2c3d4-2222-4222-8222-222222222222", "name": "Два"}
                        ]}),
                    ),
                    (
                        "predefinedItem.set",
                        json!({"at": "main:Catalog.Items.PredefinedItem.Два", "values": {"newName": "Один"}}),
                    ),
                ],
                "ops[1].args.values.newName",
            ),
        ] {
            let error = plan_all(&fixture, &operations).expect_err(path);
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{path}");
            assert_eq!(error.path(), Some(path));
        }
        // Два одноимённых элемента в исходниках: адрес ничего не выбирает,
        // и первый попавшийся не подставляется.
        assert!(unica_metadata_designer::predefined::predefined_item_id(
            br#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="00000000-0000-4000-8000-000000000001"><Name>Same</Name><Code/><Description/><IsFolder>true</IsFolder><ChildItems><Item id="00000000-0000-4000-8000-000000000002"><Name>same</Name><Code/><Description/><IsFolder>false</IsFolder></Item></ChildItems></Item></PredefinedData>"#,
            unica_metadata::MetadataKind::Catalog,
            "Same",
        )
        .is_err());
    }

    #[test]
    fn member_collection_removal_takes_the_member_from_its_address() {
        let fixture = MetadataFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let add = fixture.parse(
            "tabularSection.add",
            json!({"at": "main:Document.Order", "items": [{"name": "Строки"}]}),
            0,
        );
        let remove = fixture.parse(
            "tabularSection.remove",
            json!({"at": "main:Document.Order.TabularSection.Строки"}),
            1,
        );
        let (staged, effects) =
            plan_metadata_batch_owned_for_test(staged, authority, &[add, remove]).unwrap();
        assert_eq!(effects.len(), 2);
        let postimage = staged
            .planned_changes()
            .iter()
            .map(|change| match &change.current {
                StagedFileState::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
                StagedFileState::Absent => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !postimage.contains("Строки"),
            "removal after addition leaves no member behind"
        );
    }

    #[test]
    fn metadata_planner_preserves_operation_order_in_one_staged_postimage_without_disk_mutation() {
        let fixture = MetadataFixture::new();
        let disk_preimage = fixture.disk_bytes();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [
            fixture.parse(
                "props.set",
                json!({"at": "main:Document.Order", "values": {"Comment": "first"}}),
                0,
            ),
            fixture.parse(
                "attribute.add",
                json!({"at": "main:Document.Order", "items": [{"name": "Total"}]}),
                1,
            ),
            fixture.parse(
                "attribute.set",
                json!({
                    "at": "main:Document.Order.Attribute.Total",
                    "values": {"comment": "ordered"}
                }),
                2,
            ),
        ];

        let (staged, effects) =
            plan_metadata_batch_owned_for_test(staged, authority, &operations).unwrap();
        let changes = staged.planned_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].relative_path, Path::new("Documents/Order.xml"));
        assert_eq!(changes[0].kind, StagedChangeKind::Replace);
        assert_eq!(
            changes[0].original,
            StagedFileState::Bytes(disk_preimage.clone())
        );
        let StagedFileState::Bytes(postimage) = &changes[0].current else {
            panic!("metadata edit must stage a descriptor postimage")
        };
        let postimage = String::from_utf8(postimage.clone()).unwrap();
        assert!(postimage.contains("<Comment>first</Comment>"));
        assert!(postimage.contains("<Name>Total</Name>"));
        assert!(postimage.contains("<Comment>ordered</Comment>"));
        assert_eq!(fixture.disk_bytes(), disk_preimage);
        assert_eq!(effects.len(), 3);
        assert_eq!(effects[0].event().artifact, "Document.Order");
    }

    /// Every staged file of a plan, as text by relative path.
    fn staged_texts(
        staged: &unica_workspace::apply::ApplyStagedState,
    ) -> std::collections::BTreeMap<PathBuf, String> {
        staged
            .planned_changes()
            .into_iter()
            .filter_map(|change| match change.current {
                StagedFileState::Bytes(bytes) => {
                    Some((change.relative_path, String::from_utf8(bytes).unwrap()))
                }
                StagedFileState::Absent => None,
            })
            .collect()
    }

    /// `(lang, content)` of the multilingual property `tag` of the element
    /// whose `Name` is `name`.
    fn ml_values(text: &str, name: &str, tag: &str) -> Vec<(String, String)> {
        const CORE: &str = "http://v8.1c.ru/8.1/data/core";
        let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let properties = document
            .descendants()
            .filter(|node| node.has_tag_name("Properties"))
            .find(|node| {
                node.children()
                    .any(|child| child.has_tag_name("Name") && child.text() == Some(name))
            })
            .unwrap_or_else(|| panic!("{name} is not described\n{text}"));
        let property = properties
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == tag)
            .unwrap_or_else(|| panic!("{name} has no {tag}\n{text}"));
        property
            .children()
            .filter(|item| item.has_tag_name((CORE, "item")))
            .map(|item| {
                let child = |local: &str| {
                    item.children()
                        .find(|child| child.has_tag_name((CORE, local)))
                        .and_then(|child| child.text())
                        .unwrap_or_default()
                        .to_string()
                };
                (child("lang"), child("content"))
            })
            .collect()
    }

    #[test]
    fn exchange_content_parser_rejects_invalid_items_at_their_fields() {
        let fixture = MetadataFixture::new();
        for (items, suffix) in [
            (json!([]), "items"),
            (
                json!([{"at":"other:Catalog.Items","autoRecord":"Allow"}]),
                "items[0].at",
            ),
            (
                json!([{"at":"main:Catalog.Items.Attribute.Code","autoRecord":"Allow"}]),
                "items[0].at",
            ),
            (
                json!([{"at":"main:Catalog.Items","autoRecord":"Unknown"}]),
                "items[0].autoRecord",
            ),
            (
                json!([{"at":"main:Catalog.Items","autoRecord":"Allow","extra":true}]),
                "items[0].extra",
            ),
            (
                json!([{"at":"main:CommonModule.Shared","autoRecord":"Allow"}]),
                "items[0].at",
            ),
            (
                json!([{"at":"main:Catalog.Items","autoRecord":"Allow"},{"at":"main:ExchangePlan.Nodes","autoRecord":"Allow"}]),
                "items[1].at",
            ),
            (
                json!([{"at":"main:Catalog.Items","autoRecord":"Allow"},{"at":"main:Catalog.items","autoRecord":"Deny"}]),
                "items[1].at",
            ),
        ] {
            let error = parse_metadata_plan_operation(
                "content.set",
                &json!({"at":"main:ExchangePlan.Nodes","items":items}),
                2,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
            assert_eq!(error.path(), Some(format!("ops[2].args.{suffix}").as_str()));
        }
    }

    #[test]
    fn exchange_content_staging_reports_the_exact_owner_or_item_path() {
        let descriptor = |kind: &str, name: &str| {
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind} uuid="12345678-1234-4234-8234-123456789012"><Properties><Name>{name}</Name></Properties><ChildObjects/></{kind}></MetaDataObject>"#
            )
        };
        let content =
            r#"<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#;
        for (case, kind, suffix) in [
            ("missing_item", ApplyPlanErrorKind::NotFound, "items[1].at"),
            (
                "identity_item",
                ApplyPlanErrorKind::InvalidSource,
                "items[1].at",
            ),
            (
                "unregistered_item",
                ApplyPlanErrorKind::NotFound,
                "items[1].at",
            ),
            ("missing_owner", ApplyPlanErrorKind::NotFound, "at"),
            ("identity_owner", ApplyPlanErrorKind::InvalidSource, "at"),
            ("unregistered_owner", ApplyPlanErrorKind::NotFound, "at"),
        ] {
            let owner_registration = if case == "unregistered_owner" {
                ""
            } else {
                "<ExchangePlan>Nodes</ExchangePlan>"
            };
            let item_registration = if case == "unregistered_item" {
                ""
            } else {
                "<Catalog>Second</Catalog>"
            };
            let config = RU_CONFIGURATION_XML.replace("</ChildObjects>", &format!("{owner_registration}<Catalog>First</Catalog>{item_registration}</ChildObjects>"));
            let owner = descriptor(
                "ExchangePlan",
                if case == "identity_owner" {
                    "Other"
                } else {
                    "Nodes"
                },
            );
            let first = descriptor("Catalog", "First");
            let second = descriptor(
                "Catalog",
                if case == "identity_item" {
                    "Other"
                } else {
                    "Second"
                },
            );
            let mut files = vec![
                ("Configuration.xml", config.as_str()),
                ("Documents/Order.xml", ORDER_XML),
                ("Catalogs/First.xml", first.as_str()),
                ("ExchangePlans/Nodes/Ext/Content.xml", content),
            ];
            if case != "missing_owner" {
                files.push(("ExchangePlans/Nodes.xml", owner.as_str()));
            }
            if case != "missing_item" {
                files.push(("Catalogs/Second.xml", second.as_str()));
            }
            let fixture = MetadataFixture::with_files(&files);
            let before =
                unica_workspace::test_support::tree_snapshot(fixture.binding.source_root());
            let admission = fixture.admission();
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let operations = [fixture.parse("content.set", json!({"at":"main:ExchangePlan.Nodes","items":[{"at":"main:Catalog.First","autoRecord":"Allow"},{"at":"main:Catalog.Second","autoRecord":"Allow"}]}), 2)];
            let error = plan_metadata_batch_owned_for_test(
                admission.staged_state().unwrap(),
                authority,
                &operations,
            )
            .unwrap_err();
            assert_eq!(error.kind(), kind, "{case}: {error:?}");
            assert_eq!(
                error.path(),
                Some(format!("ops[2].args.{suffix}").as_str()),
                "{case}: {error:?}"
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(fixture.binding.source_root()),
                before,
                "{case}"
            );
        }
    }

    #[test]
    fn exchange_content_preview_uses_final_images_and_keeps_noop_out_of_events() {
        let config = RU_CONFIGURATION_XML.replace("</ChildObjects>", "<ExchangePlan>Nodes</ExchangePlan><ExchangePlan>Other</ExchangePlan><Catalog>Old</Catalog></ChildObjects>");
        let descriptor = |kind: &str, name: &str| {
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind} uuid="12345678-1234-4234-8234-123456789012"><Properties><Name>{name}</Name></Properties><ChildObjects/></{kind}></MetaDataObject>"#
            )
        };
        let content = r#"<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Item><Metadata>Catalog.Old</Metadata><AutoRecord>Deny</AutoRecord></Item></ExchangePlanContent>"#;
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", &config),
            ("Documents/Order.xml", ORDER_XML),
            (
                "ExchangePlans/Nodes.xml",
                &descriptor("ExchangePlan", "Nodes"),
            ),
            (
                "ExchangePlans/Other.xml",
                &descriptor("ExchangePlan", "Other"),
            ),
            ("Catalogs/Old.xml", &descriptor("Catalog", "Old")),
            ("ExchangePlans/Nodes/Ext/Content.xml", content),
            ("ExchangePlans/Other/Ext/Content.xml", content),
        ]);
        let admission = fixture.admission();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [("Nodes", "Allow"), ("Other", "Allow"), ("Nodes", "Deny")].into_iter().enumerate()
            .map(|(index,(name,mode))| fixture.parse("content.set", json!({"at":format!("main:ExchangePlan.{name}"),"items":[{"at":"main:Catalog.Old","autoRecord":mode}]}), index)).collect::<Vec<_>>();
        let (mut state, provisional) = plan_metadata_batch_owned_for_test(
            admission.staged_state().unwrap(),
            authority,
            &operations,
        )
        .unwrap();
        let owners = || {
            operations
                .iter()
                .filter_map(|op| op.operation().exchange_content_owner().cloned())
        };
        let details = unica_metadata_designer::planner::final_exchange_content(
            &mut state.proposal(),
            fixture.binding.source_set_name(),
            fixture.binding.source_kind(),
            owners(),
        )
        .unwrap();
        assert_eq!(
            details.iter().map(|d| d.at.as_str()).collect::<Vec<_>>(),
            ["main:ExchangePlan.Nodes", "main:ExchangePlan.Other"]
        );
        assert_eq!(
            details[0].items[0].auto_record,
            unica_metadata::MetaExchangeAutoRecord::Deny
        );
        assert_eq!(
            details[1].items[0].auto_record,
            unica_metadata::MetaExchangeAutoRecord::Allow
        );
        let effects = unica_source_contract::proposal::reconcile_effects(
            state.planned_changes(),
            provisional,
        );
        assert_eq!(effects.events().len(), 1);
        assert_eq!(effects.events()[0].artifact, "ExchangePlan.Other");
        let descriptor_path = Path::new("ExchangePlans/Nodes.xml");
        let bytes = state.read(descriptor_path).unwrap().unwrap();
        state.remove(descriptor_path, &bytes).unwrap();
        let removed = unica_metadata_designer::planner::final_exchange_content(
            &mut state.proposal(),
            fixture.binding.source_set_name(),
            fixture.binding.source_kind(),
            owners(),
        )
        .unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].at, "main:ExchangePlan.Other");
        // Finalizer must observe replacement content, even if the logical owner
        // is present again; no snapshot from the earlier set survives.
        state.create(descriptor_path, bytes).unwrap();
        let content_path = Path::new("ExchangePlans/Nodes/Ext/Content.xml");
        let before = state.read(content_path).unwrap().unwrap();
        state.replace(content_path, &before, br#"<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#.to_vec()).unwrap();
        let replaced = unica_metadata_designer::planner::final_exchange_content(
            &mut state.proposal(),
            fixture.binding.source_set_name(),
            fixture.binding.source_kind(),
            owners(),
        )
        .unwrap();
        assert!(replaced[0].items.is_empty());
        assert_eq!(
            std::fs::read_to_string(fixture.binding.source_root().join(content_path)).unwrap(),
            content
        );
    }

    #[test]
    fn exchange_content_requires_structural_configuration_registration() {
        for (registration, accepted) in [
            ("<!-- <Catalog>New</Catalog> -->", false),
            ("<Catalog xmlns='foreign'>New</Catalog>", false),
            ("<Wrapper><Catalog>New</Catalog></Wrapper>", false),
            (
                "<x:Catalog xmlns:x='http://v8.1c.ru/8.3/MDClasses'>N&#101;w</x:Catalog>",
                true,
            ),
        ] {
            let config = RU_CONFIGURATION_XML.replace(
                "</ChildObjects>",
                &format!("<ExchangePlan>Nodes</ExchangePlan>{registration}</ChildObjects>"),
            );
            let descriptor = |kind: &str, name: &str| {
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind} uuid="12345678-1234-4234-8234-123456789012"><Properties><Name>{name}</Name></Properties><ChildObjects/></{kind}></MetaDataObject>"#
                )
            };
            let content = r#"<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#;
            let fixture = MetadataFixture::with_files(&[
                ("Configuration.xml", &config),
                ("Documents/Order.xml", ORDER_XML),
                (
                    "ExchangePlans/Nodes.xml",
                    &descriptor("ExchangePlan", "Nodes"),
                ),
                ("Catalogs/New.xml", &descriptor("Catalog", "New")),
                ("ExchangePlans/Nodes/Ext/Content.xml", content),
            ]);
            let result = plan_all(
                &fixture,
                &[(
                    "content.set",
                    json!({"at":"main:ExchangePlan.Nodes", "items":[{"at":"main:Catalog.New", "autoRecord":"Allow"}]}),
                )],
            );
            assert_eq!(
                result.is_ok(),
                accepted,
                "{registration}: {:?}",
                result.as_ref().err()
            );
            assert_eq!(
                std::fs::read_to_string(
                    fixture
                        .binding
                        .source_root()
                        .join("ExchangePlans/Nodes/Ext/Content.xml")
                )
                .unwrap(),
                content
            );
        }
    }

    #[test]
    fn exchange_content_accepts_every_recordable_kind_and_refuses_others() {
        let fixture = MetadataFixture::new();
        for kind in [
            "Catalog",
            "Document",
            "Constant",
            "InformationRegister",
            "AccumulationRegister",
            "ChartOfCharacteristicTypes",
            "BusinessProcess",
            "Task",
            "ChartOfAccounts",
            "ChartOfCalculationTypes",
            "AccountingRegister",
            "CalculationRegister",
        ] {
            parse_metadata_plan_operation(
                "content.set",
                &json!({"at":"main:ExchangePlan.Nodes","items":[{"at":format!("main:{kind}.Item"),"autoRecord":"Allow"}]}),
                0,
                &fixture.binding,
            )
            .unwrap_or_else(|error| panic!("{kind}: {error:?}"));
        }
        for kind in [
            "CommonModule",
            "Enum",
            "Report",
            "Role",
            "ExchangePlan",
            "DocumentJournal",
        ] {
            let error = parse_metadata_plan_operation(
                "content.set",
                &json!({"at":"main:ExchangePlan.Nodes","items":[{"at":format!("main:{kind}.Item"),"autoRecord":"Allow"}]}),
                0,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{kind}");
            assert_eq!(error.path(), Some("ops[0].args.items[0].at"), "{kind}");
        }
    }

    #[test]
    fn exchange_content_absent_file_is_an_empty_composition_created_in_platform_layout() {
        let config = RU_CONFIGURATION_XML.replace(
            "</ChildObjects>",
            "<ExchangePlan>Nodes</ExchangePlan><Catalog>New</Catalog></ChildObjects>",
        );
        let descriptor = |kind: &str, name: &str| {
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind} uuid="12345678-1234-4234-8234-123456789012"><Properties><Name>{name}</Name></Properties><ChildObjects/></{kind}></MetaDataObject>"#
            )
        };
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", &config),
            ("Documents/Order.xml", ORDER_XML),
            (
                "ExchangePlans/Nodes.xml",
                &descriptor("ExchangePlan", "Nodes"),
            ),
            ("Catalogs/New.xml", &descriptor("Catalog", "New")),
        ]);
        let content_path = Path::new("ExchangePlans/Nodes/Ext/Content.xml");
        let admission = fixture.admission();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [fixture.parse(
            "content.set",
            json!({"at":"main:ExchangePlan.Nodes","items":[{"at":"main:Catalog.New","autoRecord":"Deny"}]}),
            0,
        )];
        let (mut state, _) = plan_metadata_batch_owned_for_test(
            admission.staged_state().unwrap(),
            authority,
            &operations,
        )
        .unwrap();
        let created = String::from_utf8(state.read(content_path).unwrap().unwrap()).unwrap();
        assert_eq!(
            created,
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<ExchangePlanContent xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\r\n\t<Item>\r\n\t\t<Metadata>Catalog.New</Metadata>\r\n\t\t<AutoRecord>Deny</AutoRecord>\r\n\t</Item>\r\n</ExchangePlanContent>"
        );
        assert!(!fixture.binding.source_root().join(content_path).exists());
        let detail = unica_metadata_designer::planner::final_exchange_content(
            &mut state.proposal(),
            fixture.binding.source_set_name(),
            fixture.binding.source_kind(),
            operations
                .iter()
                .filter_map(|op| op.operation().exchange_content_owner().cloned()),
        )
        .unwrap();
        assert_eq!(detail[0].items.len(), 1);
        assert_eq!(detail[0].items[0].at, "main:Catalog.New");
        // Without a staged file the final composition is empty, not unknown.
        let mut empty = fixture.admission().staged_state().unwrap();
        let detail = unica_metadata_designer::planner::final_exchange_content(
            &mut empty.proposal(),
            fixture.binding.source_set_name(),
            fixture.binding.source_kind(),
            operations
                .iter()
                .filter_map(|op| op.operation().exchange_content_owner().cloned()),
        )
        .unwrap();
        assert_eq!(detail.len(), 1);
        assert!(detail[0].items.is_empty());
    }

    #[test]
    fn exchange_content_saved_creation_refuses_a_file_that_appeared_before_publication() {
        let config = RU_CONFIGURATION_XML.replace(
            "</ChildObjects>",
            "<ExchangePlan>Nodes</ExchangePlan><Catalog>New</Catalog></ChildObjects>",
        );
        let descriptor = |kind: &str, name: &str| {
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind} uuid="12345678-1234-4234-8234-123456789012"><Properties><Name>{name}</Name></Properties><ChildObjects/></{kind}></MetaDataObject>"#
            )
        };
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", &config),
            ("Documents/Order.xml", ORDER_XML),
            (
                "ExchangePlans/Nodes.xml",
                &descriptor("ExchangePlan", "Nodes"),
            ),
            ("Catalogs/New.xml", &descriptor("Catalog", "New")),
        ]);
        let admission = fixture.admission();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [fixture.parse(
            "content.set",
            json!({"at":"main:ExchangePlan.Nodes","items":[{"at":"main:Catalog.New","autoRecord":"Allow"}]}),
            0,
        )];
        let (state, _) = plan_metadata_batch_owned_for_test(
            admission.staged_state().unwrap(),
            authority,
            &operations,
        )
        .unwrap();
        let prepared = admission
            .prepare_with_effects(state, Default::default())
            .unwrap();
        let token = fixture
            .actor
            .save_prepared_apply(
                prepared,
                unica_execution_contract::invocation::DomainResult::success("test preview"),
            )
            .unwrap();
        let content = fixture
            .binding
            .source_root()
            .join("ExchangePlans/Nodes/Ext/Content.xml");
        let concurrent =
            r#"<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#;
        std::fs::create_dir_all(content.parent().unwrap()).unwrap();
        std::fs::write(&content, concurrent).unwrap();
        fixture
            .actor
            .execute_saved_apply(
                &token,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
                |preview, result| {
                    result.expect_err(
                        "a plan that creates Content.xml must not overwrite a file that appeared",
                    );
                    preview
                },
            )
            .unwrap();
        assert_eq!(std::fs::read_to_string(&content).unwrap(), concurrent);
    }

    #[test]
    fn exchange_content_plan_upserts_modes_without_writing_sources() {
        let config = RU_CONFIGURATION_XML.replace("</ChildObjects>", "<ExchangePlan>Nodes</ExchangePlan><Catalog>Old</Catalog><Catalog>New</Catalog></ChildObjects>");
        let descriptor = |kind: &str, name: &str| {
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind} uuid="12345678-1234-4234-8234-123456789012"><Properties><Name>{name}</Name></Properties><ChildObjects/></{kind}></MetaDataObject>"#
            )
        };
        let content = r#"<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Item><Metadata>Catalog.Old</Metadata><AutoRecord>Deny</AutoRecord></Item></ExchangePlanContent>"#;
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", &config),
            ("Documents/Order.xml", ORDER_XML),
            (
                "ExchangePlans/Nodes.xml",
                &descriptor("ExchangePlan", "Nodes"),
            ),
            ("Catalogs/Old.xml", &descriptor("Catalog", "Old")),
            ("Catalogs/New.xml", &descriptor("Catalog", "New")),
            ("ExchangePlans/Nodes/Ext/Content.xml", content),
        ]);
        let request = |mode: &str| json!({"at":"main:ExchangePlan.Nodes", "items":[{"at":"main:Catalog.New", "autoRecord":mode}]});
        let staged = plan_all(
            &fixture,
            &[
                ("content.set", request("Allow")),
                ("content.set", request("Deny")),
            ],
        )
        .unwrap();
        let texts = staged_texts(&staged);
        let post = &texts[Path::new("ExchangePlans/Nodes/Ext/Content.xml")];
        let read = unica_metadata_designer::exchange_content::read_content(post).unwrap();
        assert!(read.errors.is_empty());
        assert_eq!(read.items.len(), 2);
        assert!(read
            .items
            .iter()
            .all(|i| i.auto_record == unica_metadata::MetaExchangeAutoRecord::Deny));
        assert_eq!(
            std::fs::read_to_string(
                fixture
                    .binding
                    .source_root()
                    .join("ExchangePlans/Nodes/Ext/Content.xml")
            )
            .unwrap(),
            content
        );
        let noop = plan_all(&fixture, &[("content.set", json!({"at":"main:ExchangePlan.Nodes", "items":[{"at":"main:Catalog.Old","autoRecord":"Deny"}]}))]).unwrap();
        assert!(noop.planned_changes().is_empty());
        let missing = plan_all(&fixture, &[("content.set", json!({"at":"main:ExchangePlan.Nodes", "items":[{"at":"main:Catalog.New","autoRecord":"Allow"},{"at":"main:Catalog.Missing","autoRecord":"Allow"}]}))]).unwrap_err();
        assert_eq!(missing.kind(), ApplyPlanErrorKind::NotFound);
        assert_eq!(
            std::fs::read_to_string(
                fixture
                    .binding
                    .source_root()
                    .join("ExchangePlans/Nodes/Ext/Content.xml")
            )
            .unwrap(),
            content
        );
        for remove in [false, true] {
            let admission = fixture.admission();
            let state = admission.staged_state().unwrap();
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let operations = [fixture.parse("content.set", request("Allow"), 0)];
            let (mut state, _) =
                plan_metadata_batch_owned_for_test(state, authority, &operations).unwrap();
            let detail = unica_metadata_designer::planner::final_exchange_content(
                &mut state.proposal(),
                fixture.binding.source_set_name(),
                fixture.binding.source_kind(),
                operations
                    .iter()
                    .filter_map(|op| op.operation().exchange_content_owner().cloned()),
            )
            .unwrap();
            assert_eq!(detail.len(), 1);
            assert_eq!(detail[0].items[0].at, "main:Catalog.Old");
            assert_eq!(detail[0].items[1].at, "main:Catalog.New");
            assert_eq!(
                detail[0].items[1].auto_record,
                unica_metadata::MetaExchangeAutoRecord::Allow
            );
            let prepared = admission
                .prepare_with_effects(state, Default::default())
                .unwrap();
            let token = fixture
                .actor
                .save_prepared_apply(
                    prepared,
                    unica_execution_contract::invocation::DomainResult::success("test preview"),
                )
                .unwrap();
            let dependency = fixture.binding.source_root().join("Catalogs/New.xml");
            if remove {
                std::fs::remove_file(&dependency).unwrap();
            } else {
                std::fs::write(
                    &dependency,
                    descriptor("Catalog", "New")
                        .replace("<ChildObjects/>", "<!--external--><ChildObjects/>"),
                )
                .unwrap();
            }
            fixture
                .actor
                .execute_saved_apply(
                    &token,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                    |preview, result| {
                        let error = result
                            .expect_err("the saved content plan must retain the target descriptor");
                        let expected = if remove {
                            unica_workspace::actor::ApplyPublicationErrorKind::ContainmentIdentity
                        } else {
                            unica_workspace::actor::ApplyPublicationErrorKind::ConcurrentRevision
                        };
                        assert_eq!(error.kind(), expected, "remove={remove}: {error}");
                        assert!(error.to_string().contains("Catalogs/New.xml"), "{error}");
                        preview
                    },
                )
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(
                    fixture
                        .binding
                        .source_root()
                        .join("ExchangePlans/Nodes/Ext/Content.xml")
                )
                .unwrap(),
                content
            );
            std::fs::write(&dependency, descriptor("Catalog", "New")).unwrap();
        }
    }

    fn plan_all(
        fixture: &MetadataFixture,
        operations: &[(&str, serde_json::Value)],
    ) -> Result<
        unica_workspace::apply::ApplyStagedState,
        unica_source_contract::proposal::ApplyPlanError,
    > {
        plan_all_with_effects(fixture, operations).map(|(staged, _)| staged)
    }

    fn plan_all_with_effects(
        fixture: &MetadataFixture,
        operations: &[(&str, serde_json::Value)],
    ) -> Result<
        (
            unica_workspace::apply::ApplyStagedState,
            unica_source_contract::proposal::PlannedApplyEffects,
        ),
        unica_source_contract::proposal::ApplyPlanError,
    > {
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let parsed = operations
            .iter()
            .enumerate()
            .map(|(index, (name, args))| fixture.parse(name, args.clone(), index))
            .collect::<Vec<_>>();
        let (staged, provisional) = plan_metadata_batch_owned_for_test(staged, authority, &parsed)?;
        let effects = unica_source_contract::proposal::reconcile_effects(
            staged.planned_changes(),
            provisional,
        );
        Ok((staged, effects))
    }

    const COMMAND_MODULE: &str =
        "\u{feff}Procedure Refresh() Export\r\n\tMessage(\"unchanged\");\r\nEndProcedure\r\n";
    const COMMAND_MODULE_RELATIVE: &str = "Documents/Order/Commands/Refresh/Ext/CommandModule.bsl";

    fn command_fixture() -> MetadataFixture {
        let order = ORDER_XML.replace(
            "<ChildObjects/>",
            concat!(
                "<ChildObjects>",
                "<Command uuid=\"22222222-2222-4222-8222-222222222222\"><Properties><Name>Refresh</Name><Synonym/><Comment>keep</Comment><Group/><Picture/><ToolTip/></Properties></Command>",
                "<Command uuid=\"33333333-3333-4333-8333-333333333333\"><Properties><Name>Inspect</Name><Synonym/><Comment/><Group/></Properties></Command>",
                "<Command uuid=\"44444444-4444-4444-8444-444444444444\"><Properties><Name>Neighbor</Name><Synonym/><Comment>neighbor</Comment><Group/></Properties></Command>",
                "</ChildObjects>"
            ),
        );
        MetadataFixture::with_files(&[
            ("Configuration.xml", RU_CONFIGURATION_XML),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", &order),
            (COMMAND_MODULE_RELATIVE, COMMAND_MODULE),
            (
                "Documents/Order/Commands/Refresh/Ext/unrelated.txt",
                "opaque neighbor",
            ),
            (
                "Documents/Order/Commands/Neighbor/Ext/CommandModule.bsl",
                "Procedure Neighbor()\nEndProcedure\n",
            ),
        ])
    }

    #[test]
    fn command_rename_stages_exact_module_bytes_and_preserves_inline_identity_and_neighbors() {
        let fixture = command_fixture();
        let original = fixture.disk_bytes();
        let staged = plan_all(
            &fixture,
            &[("command.set", json!({"at": "main:Document.Order.Command.refresh", "values": {"newName": "Updated"}}))],
        ).unwrap();
        let changes = staged.planned_changes();
        assert_eq!(
            changes.len(),
            3,
            "rename must stage owner, old module and new module"
        );
        let texts = staged_texts(&staged);
        assert_eq!(
            texts[Path::new("Documents/Order/Commands/Updated/Ext/CommandModule.bsl")],
            COMMAND_MODULE
        );
        assert!(changes.iter().any(|change| change.relative_path
            == Path::new(COMMAND_MODULE_RELATIVE)
            && change.kind == StagedChangeKind::Remove));
        let owner = &texts[Path::new("Documents/Order.xml")];
        assert_eq!(
            owner.as_bytes(),
            String::from_utf8(original.clone())
                .unwrap()
                .replace("<Name>Refresh</Name>", "<Name>Updated</Name>")
                .as_bytes()
        );
        assert_eq!(fixture.disk_bytes(), original);
        assert_eq!(
            std::fs::read(fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE)).unwrap(),
            COMMAND_MODULE.as_bytes()
        );
        assert_eq!(
            std::fs::read(
                fixture
                    .binding
                    .source_root()
                    .join("Documents/Order/Commands/Refresh/Ext/unrelated.txt")
            )
            .unwrap(),
            b"opaque neighbor"
        );
        assert!(!texts.contains_key(Path::new(
            "Documents/Order/Commands/Updated/Ext/unrelated.txt"
        )));
    }

    #[test]
    fn command_remove_stages_only_the_owner_and_its_known_module() {
        let fixture = command_fixture();
        let staged = plan_all(
            &fixture,
            &[(
                "command.remove",
                json!({"at": "main:Document.Order.Command.rEfReSh"}),
            )],
        )
        .unwrap();
        let changes = staged.planned_changes();
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().any(|change| change.relative_path
            == Path::new(COMMAND_MODULE_RELATIVE)
            && change.kind == StagedChangeKind::Remove));
        let texts = staged_texts(&staged);
        let owner = &texts[Path::new("Documents/Order.xml")];
        assert!(!owner.contains("<Name>Refresh</Name>"));
        assert!(owner.contains("<Name>Neighbor</Name>"));
        assert_eq!(
            std::fs::read(
                fixture
                    .binding
                    .source_root()
                    .join("Documents/Order/Commands/Refresh/Ext/unrelated.txt")
            )
            .unwrap(),
            b"opaque neighbor"
        );
    }

    #[test]
    fn command_property_updates_do_not_read_or_stage_the_module() {
        let fixture = command_fixture();
        let module = fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE);
        std::fs::remove_file(&module).unwrap();
        std::fs::create_dir(&module).unwrap();
        let (staged, effects) = plan_all_with_effects(&fixture, &[(
            "command.set",
            json!({"at": "main:Document.Order.Command.Refresh", "values": {"comment": "updated"}}),
        )]).unwrap();
        assert_eq!(staged.planned_changes().len(), 1);
        assert!(staged_texts(&staged)[Path::new("Documents/Order.xml")]
            .contains("<Comment>updated</Comment>"));
        assert!(
            !unica_workspace::cache::CacheImpact::from_events(effects.events())
                .invalidated
                .contains("bsl_index")
        );
        assert!(module.is_dir());
    }

    #[test]
    fn command_without_a_module_can_be_renamed_or_removed_without_materialization() {
        for (operation, values) in [
            ("command.set", json!({"newName": "Updated"})),
            ("command.remove", serde_json::Value::Null),
        ] {
            let fixture = command_fixture();
            let mut args = json!({"at": "main:Document.Order.Command.Inspect"});
            if !values.is_null() {
                args["values"] = values;
            }
            let (staged, effects) = plan_all_with_effects(&fixture, &[(operation, args)]).unwrap();
            assert_eq!(staged.planned_changes().len(), 1);
            assert!(
                !unica_workspace::cache::CacheImpact::from_events(effects.events())
                    .invalidated
                    .contains("bsl_index")
            );
            assert!(staged.planned_changes().iter().all(|change| change
                .relative_path
                .extension()
                .is_some_and(|extension| extension == "xml")));
        }
    }

    #[test]
    fn command_rename_refuses_an_orphan_destination_even_without_a_source_module() {
        for name in ["Refresh", "Inspect"] {
            let fixture = command_fixture();
            let original = fixture.disk_bytes();
            let blocked_relative = "Documents/Order/Commands/Blocked/Ext/CommandModule.bsl";
            let blocked = fixture.binding.source_root().join(blocked_relative);
            std::fs::create_dir_all(blocked.parent().unwrap()).unwrap();
            std::fs::write(&blocked, b"foreign module").unwrap();
            let error = plan_all(
                &fixture,
                &[(
                    "command.set",
                    json!({
                        "at": format!("main:Document.Order.Command.{name}"),
                        "values": {"newName": "Blocked"}
                    }),
                )],
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
            assert_eq!(error.path(), Some("ops[0].args.values.newName"));
            assert_eq!(
                error.to_string(),
                format!("the command module destination already exists: {blocked_relative}")
            );
            assert_eq!(fixture.disk_bytes(), original);
            assert_eq!(std::fs::read(&blocked).unwrap(), b"foreign module");
            assert_eq!(
                std::fs::read(fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE)).unwrap(),
                COMMAND_MODULE.as_bytes()
            );
        }
    }

    #[test]
    fn command_lifecycle_chains_use_staged_modules_and_reconcile_final_cache_effects() {
        let rename = |from: &str, to: &str| {
            (
                "command.set",
                json!({"at": format!("main:Document.Order.Command.{from}"), "values": {"newName": to}}),
            )
        };
        let remove = |name: &str| {
            (
                "command.remove",
                json!({"at": format!("main:Document.Order.Command.{name}")}),
            )
        };
        for (operations, expected_module, expected_changes) in [
            (
                vec![rename("Refresh", "Step"), rename("Step", "Final")],
                Some("Final"),
                3,
            ),
            (vec![rename("Refresh", "Step"), remove("Step")], None, 2),
            (
                vec![rename("Refresh", "Step"), rename("Step", "Refresh")],
                Some("Refresh"),
                0,
            ),
            (
                vec![
                    remove("Refresh"),
                    (
                        "command.add",
                        json!({"at": "main:Document.Order", "items": [{"name": "Refresh"}]}),
                    ),
                    rename("Refresh", "Final"),
                ],
                None,
                2,
            ),
        ] {
            let fixture = command_fixture();
            let (mut staged, effects) = plan_all_with_effects(&fixture, &operations).unwrap();
            assert_eq!(
                staged.planned_changes().len(),
                expected_changes,
                "{operations:?}"
            );
            for name in ["Refresh", "Step", "Final"] {
                let relative = format!("Documents/Order/Commands/{name}/Ext/CommandModule.bsl");
                let module = staged.read(Path::new(&relative)).unwrap();
                assert_eq!(
                    module.as_deref(),
                    (expected_module == Some(name)).then_some(COMMAND_MODULE.as_bytes()),
                    "{operations:?}, {name}"
                );
            }
            let impact = unica_workspace::cache::CacheImpact::from_events(effects.events());
            assert_eq!(
                impact.invalidated.contains("bsl_index"),
                expected_changes != 0
            );
            assert!(effects
                .events()
                .iter()
                .filter(|event| event.kind
                    == unica_source_contract::events::DomainEventKind::ModuleChanged)
                .all(|event| !event.artifact.contains(".Command.Step.")));
            if expected_changes == 0 {
                assert!(effects.events().is_empty());
            }
        }
    }

    #[test]
    fn command_lifecycle_rejects_a_stale_module_before_finalization() {
        for (operation, args) in [
            (
                "command.set",
                json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Updated"}}),
            ),
            (
                "command.remove",
                json!({"at": "main:Document.Order.Command.Refresh"}),
            ),
        ] {
            let fixture = command_fixture();
            let original = fixture.disk_bytes();
            let staged = plan_all(&fixture, &[(operation, args)]).unwrap();
            let module = fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE);
            std::fs::write(&module, b"concurrent bytes").unwrap();
            let error = staged.finalize().unwrap_err();
            assert_eq!(
                error.kind(),
                unica_source_contract::proposal::ApplyStagingErrorKind::ConcurrentRevision
            );
            assert_eq!(fixture.disk_bytes(), original);
            assert_eq!(std::fs::read(&module).unwrap(), b"concurrent bytes");
            assert!(!fixture
                .binding
                .source_root()
                .join("Documents/Order/Commands/Updated")
                .exists());
        }
    }

    #[test]
    fn command_rename_carries_the_previous_staged_module_postimage() {
        let fixture = command_fixture();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        let edited_module = b"Procedure EditedEarlier() Export\nEndProcedure\n";
        staged
            .replace(
                COMMAND_MODULE_RELATIVE,
                COMMAND_MODULE.as_bytes(),
                edited_module.to_vec(),
            )
            .unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [fixture.parse(
            "command.set",
            json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Updated"}}),
            1,
        )];
        let (mut staged, _) =
            plan_metadata_batch_owned_for_test(staged, authority, &operations).unwrap();
        assert_eq!(
            staged
                .read(Path::new(
                    "Documents/Order/Commands/Updated/Ext/CommandModule.bsl"
                ))
                .unwrap()
                .as_deref(),
            Some(edited_module.as_slice())
        );
        assert_eq!(
            staged.read(Path::new(COMMAND_MODULE_RELATIVE)).unwrap(),
            None
        );
        assert_eq!(
            std::fs::read(fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE)).unwrap(),
            COMMAND_MODULE.as_bytes()
        );
    }

    #[test]
    fn command_case_alias_rename_invalidates_the_index_and_a_roundtrip_cancels_it() {
        let rename_lower = (
            "command.set",
            json!({"at": "main:Document.Order.Command.refresh", "values": {"newName": "refresh", "comment": "keep"}}),
        );
        let fixture = command_fixture();
        let (mut staged, effects) =
            plan_all_with_effects(&fixture, std::slice::from_ref(&rename_lower)).unwrap();
        assert_eq!(
            staged
                .read(Path::new(
                    "Documents/Order/Commands/refresh/Ext/CommandModule.bsl"
                ))
                .unwrap()
                .as_deref(),
            Some(COMMAND_MODULE.as_bytes())
        );
        assert!(
            unica_workspace::cache::CacheImpact::from_events(effects.events())
                .invalidated
                .contains("bsl_index")
        );
        assert!(staged_texts(&staged)[Path::new("Documents/Order.xml")]
            .contains("<Name>refresh</Name>"));
        let (staged, effects) = plan_all_with_effects(&fixture, &[
            rename_lower,
            ("command.set", json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Refresh", "comment": "keep"}})),
        ]).unwrap();
        assert!(staged.planned_changes().is_empty());
        assert!(effects.events().is_empty());
        staged.finalize().unwrap();
    }

    #[test]
    fn command_case_correction_to_the_existing_physical_spelling_publishes_and_invalidates() {
        let fixture = command_fixture();
        let lowercase_module = fixture
            .binding
            .source_root()
            .join("Documents/Order/Commands/refresh/Ext/CommandModule.bsl");
        let case_alias = lowercase_module.is_file();
        let lowercase_owner = String::from_utf8(fixture.disk_bytes())
            .unwrap()
            .replace("<Name>Refresh</Name>", "<Name>refresh</Name>");
        std::fs::write(&fixture.descriptor, lowercase_owner).unwrap();
        let admission = fixture
            .actor
            .admit_apply(
                &fixture.binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [fixture.parse(
            "command.set",
            json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Refresh", "comment": "keep"}}),
            0,
        )];
        let planned = plan_metadata_batch_owned_for_test(staged, authority, &operations);
        if case_alias {
            let (staged, provisional) = planned.unwrap();
            let effects = unica_source_contract::proposal::reconcile_effects(
                staged.planned_changes(),
                provisional,
            );
            let prepared = admission.prepare_with_effects(staged, effects).unwrap();
            let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
            assert!(result
                .effects()
                .cache()
                .invalidated
                .iter()
                .any(|name| name == "bsl_index"));
            assert!(String::from_utf8(fixture.disk_bytes())
                .unwrap()
                .contains("<Name>Refresh</Name>"));
            assert_eq!(
                std::fs::read(fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE)).unwrap(),
                COMMAND_MODULE.as_bytes()
            );
        } else {
            assert_eq!(
                planned.unwrap_err().kind(),
                ApplyPlanErrorKind::InvalidState
            );
        }
    }

    #[test]
    fn command_case_alias_rename_invalidates_after_an_earlier_aliased_read() {
        let fixture = command_fixture();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        staged
            .read(Path::new(
                "Documents/Order/Commands/rEfReSh/Ext/CommandModule.bsl",
            ))
            .unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [fixture.parse(
            "command.set",
            json!({"at": "main:Document.Order.Command.refresh", "values": {"newName": "refresh", "comment": "keep"}}),
            1,
        )];
        let (staged, provisional) =
            plan_metadata_batch_owned_for_test(staged, authority, &operations).unwrap();
        let effects = unica_source_contract::proposal::reconcile_effects(
            staged.planned_changes(),
            provisional,
        );
        assert!(
            unica_workspace::cache::CacheImpact::from_events(effects.events())
                .invalidated
                .contains("bsl_index")
        );
    }

    #[test]
    fn command_lifecycle_effects_follow_the_retained_spelling_of_earlier_reads() {
        for (operation, args) in [
            (
                "command.set",
                json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Updated"}}),
            ),
            (
                "command.remove",
                json!({"at": "main:Document.Order.Command.Refresh"}),
            ),
        ] {
            let fixture = command_fixture();
            let admission = fixture.admission();
            let mut staged = admission.staged_state().unwrap();
            for relative in [
                "Documents/Order/Commands/rEfReSh/Ext/CommandModule.bsl",
                "Documents/Order/Commands/uPdAtEd/Ext/CommandModule.bsl",
            ] {
                staged.read(Path::new(relative)).unwrap();
            }
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let operations = [fixture.parse(operation, args, 1)];
            let (staged, provisional) =
                plan_metadata_batch_owned_for_test(staged, authority, &operations).unwrap();
            let effects = unica_source_contract::proposal::reconcile_effects(
                staged.planned_changes(),
                provisional,
            );
            assert!(
                unica_workspace::cache::CacheImpact::from_events(effects.events())
                    .invalidated
                    .contains("bsl_index"),
                "{operation}"
            );
        }
    }

    #[test]
    fn command_lifecycle_refuses_links_at_the_source_or_destination_module_route() {
        use unica_workspace::test_support::{
            create_directory_link_fixture_for_test, create_file_link_fixture_for_test,
            FileLinkFixtureOutcome,
        };
        for (source, directory) in [(true, false), (false, false), (true, true), (false, true)] {
            let fixture = command_fixture();
            let original = fixture.disk_bytes();
            let outside = tempfile::tempdir().unwrap();
            let foreign = outside.path().join("CommandModule.bsl");
            std::fs::write(&foreign, b"foreign module").unwrap();
            let name = if source { "Refresh" } else { "Updated" };
            let ext = fixture
                .binding
                .source_root()
                .join(format!("Documents/Order/Commands/{name}/Ext"));
            if source {
                if directory {
                    std::fs::rename(&ext, ext.with_file_name("ExtSaved")).unwrap();
                } else {
                    std::fs::remove_file(ext.join("CommandModule.bsl")).unwrap();
                }
            } else {
                std::fs::create_dir_all(ext.parent().unwrap()).unwrap();
                if !directory {
                    std::fs::create_dir(&ext).unwrap();
                }
            }
            let outcome = if directory {
                create_directory_link_fixture_for_test(outside.path(), &ext)
            } else {
                create_file_link_fixture_for_test(&foreign, ext.join("CommandModule.bsl"))
            }
            .unwrap();
            if outcome != FileLinkFixtureOutcome::Created {
                continue;
            }
            let error = plan_all(&fixture, &[("command.set", json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Updated"}}))]).unwrap_err();
            use unica_source_contract::proposal::ApplyStagingErrorKind;
            assert!(
                matches!(
                    error.kind(),
                    ApplyPlanErrorKind::Staging(
                        ApplyStagingErrorKind::ContainmentIdentity
                            | ApplyStagingErrorKind::UnsupportedProvider
                    )
                ),
                "{source}, {directory}: {error}"
            );
            assert!(error.to_string().contains("link"), "{error}");
            assert_eq!(
                error.path(),
                Some(if source {
                    "ops[0].args.at"
                } else {
                    "ops[0].args.values.newName"
                })
            );
            assert_eq!(fixture.disk_bytes(), original);
            assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign module");
        }
    }

    const KINDS_ENUM_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Enum uuid="66666666-6666-4666-8666-666666666666">
		<Properties>
			<Name>Kinds</Name>
			<Synonym/>
			<Comment/>
		</Properties>
		<ChildObjects/>
	</Enum>
</MetaDataObject>
"#;

    const READER_ROLE_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<Role uuid="77777777-7777-4777-8777-777777777777">
		<Properties>
			<Name>Reader</Name>
			<Synonym/>
			<Comment/>
		</Properties>
	</Role>
</MetaDataObject>
"#;

    const SALES_SUBSYSTEM_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<Subsystem uuid="88888888-8888-4888-8888-888888888888">
		<Properties>
			<Name>Sales</Name>
			<Synonym/>
			<Comment/>
			<IncludeHelpInContents>true</IncludeHelpInContents>
			<IncludeInCommandInterface>true</IncludeInCommandInterface>
			<UseOneCommand>false</UseOneCommand>
			<Explanation/>
			<Picture/>
			<Content/>
		</Properties>
		<ChildObjects/>
	</Subsystem>
</MetaDataObject>
"#;

    #[test]
    fn properties_typed_patches_compose_and_preserve_descriptor_companions() {
        let retained_translation =
            "<v8:item><v8:lang>en</v8:lang><v8:content>Retained translation</v8:content></v8:item>";
        let role = READER_ROLE_XML.replace(
            "<Synonym/>",
            &format!("<Synonym>{retained_translation}</Synonym>"),
        );
        let subsystem = SALES_SUBSYSTEM_XML.replace(
            "<Synonym/>",
            &format!("<Synonym>{retained_translation}</Synonym>"),
        );
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", RU_CONFIGURATION_XML),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", ORDER_XML),
            ("Roles/Reader.xml", &role),
            (
                "Roles/Reader/Ext/Rights.xml",
                "<Rights>retained role content</Rights>",
            ),
            ("Subsystems/Sales.xml", &subsystem),
            (
                "Subsystems/Sales/Ext/CommandInterface.xml",
                "<CommandInterface>retained interface</CommandInterface>",
            ),
            (
                "Documents/Order/Ext/ObjectModule.bsl",
                "// retained module\r\n",
            ),
        ]);
        let before = unica_workspace::test_support::tree_snapshot(fixture.binding.source_root());
        let staged = plan_all(&fixture, &[
            ("props.set", json!({"at":"main:Document.Order","values":{"Comment":"first"}})),
            ("attribute.add", json!({"at":"main:Document.Order","items":[{"name":"Total"}]})),
            ("props.set", json!({"at":"main:Document.Order","values":{"Comment":"final"}})),
            ("props.set", json!({"at":"main:Role.Reader","values":{"synonym":"Reader edited","Comment":"clear later"}})),
            ("props.set", json!({"at":"main:Role.Reader","values":{"comment":""}})),
            ("props.set", json!({"at":"main:Subsystem.Sales","values":{
                "synonym":"Sales edited","comment":"first","Explanation":"About sales",
                "IncludeHelpInContents":false,"includeInCommandInterface":false,"useOneCommand":true
            }})),
            ("props.set", json!({"at":"main:Subsystem.Sales","values":{"Comment":"last"}})),
        ]).unwrap();
        let texts = staged_texts(&staged);
        assert_eq!(
            texts.keys().cloned().collect::<Vec<_>>(),
            vec![
                PathBuf::from("Documents/Order.xml"),
                PathBuf::from("Roles/Reader.xml"),
                PathBuf::from("Subsystems/Sales.xml"),
            ]
        );
        let order = &texts[Path::new("Documents/Order.xml")];
        assert!(order.contains("<Comment>final</Comment>"));
        assert!(order.contains("<Name>Total</Name>"));
        let role = &texts[Path::new("Roles/Reader.xml")];
        assert!(role.contains("<Comment/>"));
        assert!(role.contains(retained_translation));
        assert!(
            ml_values(role, "Reader", "Synonym").contains(&("ru".into(), "Reader edited".into()))
        );
        let sales = &texts[Path::new("Subsystems/Sales.xml")];
        assert!(sales.contains("<Comment>last</Comment>"));
        assert!(sales.contains("<IncludeHelpInContents>false</IncludeHelpInContents>"));
        assert!(sales.contains("<IncludeInCommandInterface>false</IncludeInCommandInterface>"));
        assert!(sales.contains("<UseOneCommand>true</UseOneCommand>"));
        assert!(sales.contains("<Picture/>"));
        assert!(sales.contains("<Content/>"));
        assert!(sales.contains(retained_translation));
        assert!(
            ml_values(sales, "Sales", "Synonym").contains(&("ru".into(), "Sales edited".into()))
        );
        assert_eq!(
            ml_values(sales, "Sales", "Explanation"),
            vec![("ru".into(), "About sales".into())]
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(fixture.binding.source_root()),
            before
        );
    }

    #[test]
    fn properties_argument_errors_precede_missing_or_damaged_descriptor_state() {
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", RU_CONFIGURATION_XML),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", ORDER_XML),
            ("Roles/Reader.xml", READER_ROLE_XML),
            ("Subsystems/Sales.xml", "<"),
        ]);
        std::fs::write(
            fixture.binding.source_root().join("Roles/Reader.xml"),
            [0xff],
        )
        .unwrap();
        let before = unica_workspace::test_support::tree_snapshot(fixture.binding.source_root());
        for (at, expected_state) in [
            ("main:Role.Missing", ApplyPlanErrorKind::NotFound),
            ("main:Role.Reader", ApplyPlanErrorKind::InvalidSource),
            ("main:Subsystem.Sales", ApplyPlanErrorKind::InvalidSource),
        ] {
            for (values, path) in [
                (json!({"comment":false}), "ops[5].args.values.comment"),
                (json!({}), "ops[5].args.values"),
                (
                    json!({"Comment":"a","comment":"a"}),
                    "ops[5].args.values.comment",
                ),
            ] {
                let error = parse_metadata_plan_operation(
                    "props.set",
                    &json!({"at":at,"values":values}),
                    5,
                    &fixture.binding,
                )
                .unwrap_err();
                assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{at}: {error}");
                assert_eq!(error.path(), Some(path), "{at}: {error}");
            }
            let admission = fixture.admission();
            let parsed = fixture.parse(
                "props.set",
                json!({"at":at,"values":{"comment":"valid"}}),
                5,
            );
            let error = plan_metadata_batch_owned_for_test(
                admission.staged_state().unwrap(),
                admission
                    .metadata_planning_authority(&fixture.binding)
                    .unwrap(),
                &[parsed],
            )
            .unwrap_err();
            assert_eq!(error.kind(), expected_state, "{at}: {error}");
            assert_eq!(error.path(), Some("ops[5].args.at"));
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(fixture.binding.source_root()),
                before
            );
        }
    }

    #[test]
    fn properties_late_failure_and_stale_inputs_publish_no_partial_patch() {
        for relative in [
            "Configuration.xml",
            "Languages/Русский.xml",
            "Roles/Reader.xml",
        ] {
            let fixture = MetadataFixture::with_files(&[
                ("Configuration.xml", RU_CONFIGURATION_XML),
                ("Languages/Русский.xml", &language_xml("Русский", "ru")),
                ("Documents/Order.xml", ORDER_XML),
                ("Roles/Reader.xml", READER_ROLE_XML),
            ]);
            let before =
                unica_workspace::test_support::tree_snapshot(fixture.binding.source_root());
            let admission = fixture.admission();
            let error = plan_metadata_batch_owned_for_test(
                admission.staged_state().unwrap(),
                admission
                    .metadata_planning_authority(&fixture.binding)
                    .unwrap(),
                &[
                    fixture.parse(
                        "props.set",
                        json!({"at":"main:Role.Reader","values":{"synonym":"not published"}}),
                        0,
                    ),
                    fixture.parse(
                        "attribute.remove",
                        json!({"at":"main:Document.Order.Attribute.Missing"}),
                        1,
                    ),
                ],
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound);
            assert_eq!(error.path(), Some("ops[1].args.at"));
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(fixture.binding.source_root()),
                before
            );
            let staged = plan_all(
                &fixture,
                &[(
                    "props.set",
                    json!({"at":"main:Role.Reader","values":{"synonym":"not published"}}),
                )],
            )
            .unwrap();
            let path = fixture.binding.source_root().join(relative);
            let mut changed = std::fs::read(&path).unwrap();
            changed.extend_from_slice(b"<!-- concurrent edit -->");
            std::fs::write(path, changed).unwrap();
            let concurrent =
                unica_workspace::test_support::tree_snapshot(fixture.binding.source_root());
            let error = staged.finalize().unwrap_err();
            assert_eq!(
                error.kind(),
                unica_source_contract::proposal::ApplyStagingErrorKind::ConcurrentRevision,
                "{relative}"
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(fixture.binding.source_root()),
                concurrent
            );
        }
    }

    #[test]
    fn new_multilingual_values_take_the_configuration_default_language() {
        let fixture = MetadataFixture::with_files(&[
            (
                "Configuration.xml",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.English</DefaultLanguage></Properties><ChildObjects><Language>English</Language><Document>Order</Document><Enum>Kinds</Enum><Role>Reader</Role><Subsystem>Sales</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
            ),
            ("Languages/English.xml", &language_xml("English", "en")),
            ("Documents/Order.xml", ORDER_XML),
            ("Enums/Kinds.xml", KINDS_ENUM_XML),
            ("Roles/Reader.xml", READER_ROLE_XML),
            ("Subsystems/Sales.xml", SALES_SUBSYSTEM_XML),
        ]);
        let staged = plan_all(
            &fixture,
            &[
                (
                    "props.set",
                    json!({"at": "main:Document.Order", "values": {"Synonym": "Sales order"}}),
                ),
                (
                    "attribute.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Total"}, {"name": "Note"}]}),
                ),
                (
                    "attribute.set",
                    json!({"at": "main:Document.Order.Attribute.Total", "values": {"synonym": "Grand total"}}),
                ),
                (
                    "tabularSection.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Lines", "attributes": [{"name": "Qty"}]}]}),
                ),
                (
                    "attribute.add",
                    json!({"at": "main:Document.Order.TabularSection.Lines", "items": [{"name": "Price"}]}),
                ),
                (
                    "command.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Print"}]}),
                ),
                (
                    "template.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Layout", "templateType": "SpreadsheetDocument"}]}),
                ),
                (
                    "enumValue.add",
                    json!({"at": "main:Enum.Kinds", "items": [{"name": "Retail"}]}),
                ),
                (
                    "props.set",
                    json!({"at": "main:Role.Reader", "values": {"synonym": "Reader"}}),
                ),
                (
                    "props.set",
                    json!({"at": "main:Subsystem.Sales", "values": {"synonym": "Sales", "explanation": "Sales area"}}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        let text = |relative: &str| {
            texts
                .get(Path::new(relative))
                .unwrap_or_else(|| panic!("{relative} is not staged"))
                .as_str()
        };
        let english = |content: &str| vec![("en".to_string(), content.to_string())];
        let order = text("Documents/Order.xml");
        assert_eq!(ml_values(order, "Order", "Synonym"), english("Sales order"));
        assert_eq!(ml_values(order, "Total", "Synonym"), english("Grand total"));
        assert_eq!(ml_values(order, "Note", "Synonym"), english("Note"));
        assert_eq!(ml_values(order, "Lines", "Synonym"), english("Lines"));
        assert_eq!(ml_values(order, "Qty", "Synonym"), english("Qty"));
        assert_eq!(ml_values(order, "Price", "Synonym"), english("Price"));
        assert_eq!(ml_values(order, "Print", "Synonym"), english("Print"));
        assert_eq!(
            ml_values(
                text("Documents/Order/Templates/Layout.xml"),
                "Layout",
                "Synonym"
            ),
            english("Layout")
        );
        assert_eq!(
            ml_values(text("Enums/Kinds.xml"), "Retail", "Synonym"),
            english("Retail")
        );
        assert_eq!(
            ml_values(text("Roles/Reader.xml"), "Reader", "Synonym"),
            english("Reader")
        );
        let sales = text("Subsystems/Sales.xml");
        assert_eq!(ml_values(sales, "Sales", "Synonym"), english("Sales"));
        assert_eq!(
            ml_values(sales, "Sales", "Explanation"),
            english("Sales area")
        );
        for (relative, text) in &texts {
            assert!(
                !text.contains("<v8:lang>ru</v8:lang>"),
                "{} invents a ru value\n{text}",
                relative.display()
            );
        }
    }

    const ENGLISH_ONLY_CONFIGURATION_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.English</DefaultLanguage></Properties><ChildObjects><Language>English</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#;

    #[test]
    fn object_create_writes_the_configuration_default_language() {
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", ENGLISH_ONLY_CONFIGURATION_XML),
            ("Languages/English.xml", &language_xml("English", "en")),
            ("Documents/Order.xml", ORDER_XML),
        ]);
        let kinds = [
            ("Catalog", "Catalogs", "Products"),
            ("Document", "Documents", "Invoice"),
            ("Enum", "Enums", "Statuses"),
            ("InformationRegister", "InformationRegisters", "Prices"),
            ("CommonModule", "CommonModules", "Helpers"),
            ("Report", "Reports", "Sales"),
        ];
        let operations = kinds
            .iter()
            .map(|(kind, _, name)| {
                (
                    "object.create",
                    json!({"at": "main:Configuration", "values": {"kind": kind, "name": name}}),
                )
            })
            .collect::<Vec<_>>();
        let staged = plan_all(&fixture, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        for (kind, directory, name) in kinds {
            let relative = format!("{directory}/{name}.xml");
            let text = texts
                .get(Path::new(&relative))
                .unwrap_or_else(|| panic!("{kind}: {relative} is not staged"));
            assert_eq!(
                ml_values(text, name, "Synonym"),
                vec![("en".to_string(), name.to_string())],
                "{kind}"
            );
        }
        for (relative, text) in &texts {
            assert!(
                !text.contains("<v8:lang>ru</v8:lang>"),
                "{} invents a ru value\n{text}",
                relative.display()
            );
        }

        let refused = MetadataFixture::with_files(&[
            (
                "Configuration.xml",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
            ),
            ("Documents/Order.xml", ORDER_XML),
        ]);
        let error = plan_all(
            &refused,
            &[(
                "object.create",
                json!({"at": "main:Configuration", "values": {"kind": "Catalog", "name": "Products"}}),
            )],
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
        assert_eq!(error.path(), Some("ops[0].args.at"));
    }

    #[test]
    fn an_extension_writes_new_values_in_its_own_default_language() {
        // The extension's own Configuration.xml and adopted language decide;
        // nothing is read from the configuration it extends.
        let configuration = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="99999999-9999-4999-8999-999999999999"><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>Ext</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose><NamePrefix>Ext_</NamePrefix><DefaultLanguage>Language.English</DefaultLanguage></Properties><ChildObjects><Language>English</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#;
        let language = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><InternalInfo/><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>English</Name><Comment/><ExtendedConfigurationObject>44444444-4444-4444-8444-444444444444</ExtendedConfigurationObject><LanguageCode>en</LanguageCode></Properties></Language></MetaDataObject>"#;
        let fixture = MetadataFixture::with_kind_and_files(
            SourceSetKind::Extension,
            &[
                ("Configuration.xml", configuration),
                ("Languages/English.xml", language),
                ("Documents/Order.xml", ORDER_XML),
            ],
        );
        let staged = plan_all(
            &fixture,
            &[
                (
                    "object.create",
                    json!({"at": "main:Configuration", "values": {"kind": "Catalog", "name": "Ext_Products"}}),
                ),
                (
                    "attribute.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Total"}]}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        assert_eq!(
            ml_values(
                &texts[Path::new("Catalogs/Ext_Products.xml")],
                "Ext_Products",
                "Synonym"
            ),
            vec![("en".to_string(), "Ext_products".to_string())]
        );
        assert_eq!(
            ml_values(&texts[Path::new("Documents/Order.xml")], "Total", "Synonym"),
            vec![("en".to_string(), "Total".to_string())]
        );
    }

    #[test]
    fn editing_a_multilingual_value_changes_only_the_default_language_item() {
        const WORKSPACE: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/acceptance/workspace/src"
        );
        let read =
            |relative: &str| std::fs::read_to_string(Path::new(WORKSPACE).join(relative)).unwrap();
        let language_aware = read("Enums/LanguageAware.xml");
        let item = |lang: &str, content: &str, indent: &str| {
            format!(
                "{indent}<v8:item>\n{indent}\t<v8:lang>{lang}</v8:lang>\n{indent}\t<v8:content>{content}</v8:content>\n{indent}</v8:item>\n"
            )
        };
        let bilingual = |name: &str, ru: &str, en: &str| {
            let indent = "\t\t\t\t\t\t\t";
            format!(
                "<Properties><Name>{name}</Name><Synonym>\n{}{}\t\t\t\t\t\t</Synonym><Comment/>",
                item("ru", ru, indent),
                item("en", en, indent)
            )
        };
        let order = ORDER_XML.replace(
            "<ChildObjects/>\n\t</Document>",
            &format!(
                "<ChildObjects><Attribute uuid=\"22222222-2222-4222-8222-222222222222\">{}</Properties></Attribute><TabularSection uuid=\"33333333-3333-4333-8333-333333333334\"><Properties><Name>Lines</Name><Synonym/><Comment/></Properties><ChildObjects><Attribute uuid=\"44444444-4444-4444-8444-444444444445\">{}</Properties></Attribute></ChildObjects></TabularSection></ChildObjects>\n\t</Document>",
                bilingual("Total", "Итого", "Total"),
                bilingual("Qty", "Количество", "Quantity"),
            ),
        );
        let kinds = KINDS_ENUM_XML.replace(
            "<ChildObjects/>",
            &format!(
                "<ChildObjects><EnumValue uuid=\"55555555-5555-4555-8555-555555555557\">{}</Properties></EnumValue></ChildObjects>",
                bilingual("Retail", "Розница", "Retail")
            ),
        );
        let block = |tag: &str, ru: &str, en: &str| {
            format!(
                "<{tag}>\n{}{}\t\t\t</{tag}>",
                item("ru", ru, "\t\t\t\t"),
                item("en", en, "\t\t\t\t")
            )
        };
        let sales = SALES_SUBSYSTEM_XML
            .replace("<Synonym/>", &block("Synonym", "Продажи", "Sales"))
            .replace(
                "<Explanation/>",
                &block("Explanation", "Раздел продаж", "Sales area"),
            )
            .replace(
                "<Content/>",
                "<Content>\n\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">Document.Order</xr:Item>\n\t\t\t</Content>",
            )
            .replace(
                "xmlns:xr=",
                "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xr=",
            );
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", &read("Configuration.xml")),
            ("Languages/Русский.xml", &read("Languages/Русский.xml")),
            ("Languages/English.xml", &read("Languages/English.xml")),
            ("Enums/LanguageAware.xml", &language_aware),
            ("Enums/Kinds.xml", &kinds),
            ("Documents/Order.xml", &order),
            ("Subsystems/Sales.xml", &sales),
        ]);
        let staged = plan_all(
            &fixture,
            &[
                (
                    "props.set",
                    json!({"at": "main:Enum.LanguageAware", "values": {"Synonym": "Короткое & новое"}}),
                ),
                (
                    "attribute.set",
                    json!({"at": "main:Document.Order.Attribute.Total", "values": {"synonym": "Всего"}}),
                ),
                (
                    "attribute.set",
                    json!({"at": "main:Document.Order.TabularSection.Lines.Attribute.Qty", "values": {"synonym": "Кол-во"}}),
                ),
                (
                    "enumValue.set",
                    json!({"at": "main:Enum.Kinds.EnumValue.Retail", "values": {"synonym": "Розничная"}}),
                ),
                (
                    "props.set",
                    json!({"at": "main:Subsystem.Sales", "values": {
                        "synonym": "Продажи и маркетинг",
                        "explanation": "Новый раздел",
                        "comment": "Отдел"
                    }}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        // The default language is Русский (`ru`): its content is the only
        // change, and the English items stay byte for byte.
        assert_eq!(
            texts[Path::new("Enums/LanguageAware.xml")],
            language_aware.replace(
                "Очень длинное наименование перечисления для интерфейса команд",
                "Короткое &amp; новое"
            )
        );
        assert_eq!(
            texts[Path::new("Documents/Order.xml")],
            order
                .replace("Итого", "Всего")
                .replace("Количество", "Кол-во")
        );
        assert_eq!(
            texts[Path::new("Enums/Kinds.xml")],
            kinds.replace("Розница", "Розничная")
        );
        // The subsystem is edited in place: English, Picture and Content
        // keep their bytes.
        assert_eq!(
            texts[Path::new("Subsystems/Sales.xml")],
            sales
                .replace(
                    "<v8:content>Продажи</v8:content>",
                    "<v8:content>Продажи и маркетинг</v8:content>"
                )
                .replace("Раздел продаж", "Новый раздел")
                .replace("<Comment/>", "<Comment>Отдел</Comment>")
        );
    }

    #[test]
    fn a_configuration_without_a_default_language_refuses_new_text_but_not_other_edits() {
        let fixture = MetadataFixture::with_files(&[
            (
                "Configuration.xml",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
            ),
            ("Documents/Order.xml", ORDER_XML),
        ]);
        let error = plan_all(
            &fixture,
            &[(
                "attribute.add",
                json!({"at": "main:Document.Order", "items": [{"name": "Total"}]}),
            )],
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
        assert_eq!(error.path(), Some("ops[0].args.at"));
        assert_eq!(fixture.disk_bytes(), ORDER_XML.as_bytes());

        plan_all(
            &fixture,
            &[(
                "props.set",
                json!({"at": "main:Document.Order", "values": {"Comment": "typed"}}),
            )],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
    }

    #[test]
    fn multilingual_property_edits_keep_the_other_languages() {
        let text = "<Properties xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">\n\t\t\t<Name>SalesReader</Name>\n\t\t\t<Synonym>\n\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>en</v8:lang>\n\t\t\t\t\t<v8:content>Sales reader</v8:content>\n\t\t\t\t</v8:item>\n\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>ru</v8:lang>\n\t\t\t\t\t<v8:content>Старое</v8:content>\n\t\t\t\t</v8:item>\n\t\t\t</Synonym>\n\t\t\t<Comment/>\n\t\t</Properties>";
        let updated = set_ml_property(text, "Synonym", "\t\t\t", "ru", "Новое & лучшее").unwrap();
        assert!(updated.contains("<v8:content>Sales reader</v8:content>"));
        assert!(updated.contains("<v8:content>Новое &amp; лучшее</v8:content>"));
        assert!(!updated.contains("Старое"));
        assert_eq!(updated.matches("<v8:item>").count(), 2);

        let english_only = text.replace(
            "\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>ru</v8:lang>\n\t\t\t\t\t<v8:content>Старое</v8:content>\n\t\t\t\t</v8:item>\n",
            "",
        );
        let updated = set_ml_property(&english_only, "Synonym", "\t\t\t", "ru", "Русское").unwrap();
        assert!(updated.contains("<v8:content>Sales reader</v8:content>"));
        assert!(
            updated.contains("<v8:lang>ru</v8:lang>\n\t\t\t\t\t<v8:content>Русское</v8:content>")
        );
        assert_eq!(updated.matches("<v8:item>").count(), 2);

        let empty = "<Properties xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">\n\t\t\t<Name>X</Name>\n\t\t\t<Synonym/>\n\t\t</Properties>";
        let updated = set_ml_property(empty, "Synonym", "\t\t\t", "ru", "Икс").unwrap();
        assert!(updated.contains("<Synonym>\n\t\t\t\t<v8:item>"));
        assert!(updated.contains("<v8:content>Икс</v8:content>"));
        assert!(!updated.contains("<Synonym/>"));
    }

    #[test]
    fn descriptor_images_keep_the_observed_bom_and_line_endings() {
        let source = "<a>\r\n\t<b/>\r\n</a>\r\n";
        let preimage = [b"\xef\xbb\xbf".as_slice(), source.as_bytes()].concat();
        let image = unica_source_designer::registration::preserve_descriptor_image(
            &preimage,
            source,
            "<a>\r\n\t<b/>\n\t<c/>\r\n</a>\r\n",
        );
        assert!(image.starts_with(b"\xef\xbb\xbf"));
        assert_eq!(&image[3..], b"<a>\r\n\t<b/>\r\n\t<c/>\r\n</a>\r\n");
        let plain = unica_source_designer::registration::preserve_descriptor_image(
            source.as_bytes(),
            source,
            "<a>\n</a>\n",
        );
        assert!(!plain.starts_with(b"\xef\xbb\xbf"));
    }
}
