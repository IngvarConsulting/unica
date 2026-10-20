use unica_metadata_designer::validation::*;

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(test)]
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
#[cfg(test)]
use unica_metadata::{MetaDiagnostic, MetaDiagnosticSeverity, MetadataKind};
use unica_source_contract::workspace::WorkspaceContext;

use super::super::cf::validate_cf_owner_path;
use super::super::common::read_utf8_sig;
use crate::infrastructure::subsystem_composition::validate_subsystem_owner_path;

#[cfg(test)]
const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

pub(crate) fn validate_metadata_owner_shape_8_3_27(
    object_path: &Path,
    workspace: &WorkspaceContext,
    operation: &str,
) -> Result<(), String> {
    let xml_text = read_utf8_sig(object_path)?;
    match unica_metadata_designer::format_contract::metadata_owner_object_name(&xml_text)?.as_str()
    {
        "Configuration" => return validate_cf_owner_path(object_path, workspace),
        "Subsystem" => return validate_subsystem_owner_path(object_path, workspace),
        _ => {}
    }
    unica_metadata_designer::format_contract::validate_metadata_8_3_27_boolean_contract(
        &xml_text, operation,
    )?;
    unica_metadata_designer::format_contract::validate_metadata_8_3_27_enum_contract(
        &xml_text, operation,
    )?;

    let run = validate_local_metadata_image(
        object_path.to_path_buf(),
        &MetaValidationOptions { max_errors: 30 },
        workspace,
    )?;
    if run.ok {
        Ok(())
    } else {
        Err(format!(
            "{operation} owner metadata validation failed for {}: {}",
            object_path.display(),
            run.errors.join("; ")
        ))
    }
}

fn validate_local_metadata_image(
    raw_path: PathBuf,
    options: &MetaValidationOptions,
    context: &WorkspaceContext,
) -> Result<MetaValidationRun, String> {
    let object_path = unica_metadata_designer::info_read::resolve_meta_info_path(
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&context.cwd),
        unica_operation_contract::input::absolutize(raw_path, &context.cwd),
    )?;
    let resolved_path = object_path
        .canonicalize()
        .unwrap_or_else(|_| object_path.clone());
    let text = read_utf8_sig(&resolved_path)?;
    let reference_inputs = MetaValidationReferenceInputs {
        config_dir: None,
        language_codes: Vec::new(),
        proof_subject: None,
    };
    meta_validate_source(text.as_bytes(), options, &reference_inputs)
}

// Shared core of the service semantics check: meta validation reports it as
// check 11 for a standalone object, cfe validation as check 14 across the
// extension's child objects. Returns None for non-service metadata types.

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use unica_metadata::subsystem::SubsystemAddress;
    use unica_metadata::{MetaDiagnosticCode, MetaValidationStatus};

    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    /// `AutomaticAndManaged` допустим только у корня конфигурации: корень
    /// уходит в проверку `cf` раньше общих перечислений объектов (#1271).
    #[test]
    fn configuration_accepts_automatic_and_managed_lock_mode_unlike_objects() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let workspace = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 0,
        };
        let init = super::super::super::cf::create_configuration_scaffold(
            serde_json::json!({"Name": "Demo", "OutputDir": "src"})
                .as_object()
                .unwrap(),
            &workspace,
        );
        assert!(init.ok, "{init:?}");
        let config_path = root.join("src/Configuration.xml");
        let original = fs::read_to_string(&config_path).unwrap();
        let mixed = original.replacen(
            "<DataLockControlMode>Managed</DataLockControlMode>",
            "<DataLockControlMode>AutomaticAndManaged</DataLockControlMode>",
            1,
        );
        assert_ne!(mixed, original, "the scaffold must declare a lock mode");
        fs::write(&config_path, &mixed).unwrap();

        let owner = validate_metadata_owner_shape_8_3_27(&config_path, &workspace, "test");
        assert!(owner.is_ok(), "{owner:?}");
        let check = super::super::super::cf::validate_cf(
            &serde_json::Map::from_iter([(
                "ConfigPath".to_string(),
                serde_json::Value::from("src"),
            )]),
            &workspace,
        );
        assert!(check.ok, "{check:?}");
        drop(temp);

        let (catalog, _) =
            unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                MetadataKind::Catalog,
                "Demo",
            )
            .unwrap();
        let mixed_catalog = catalog.replacen(
            "<DataLockControlMode>Automatic</DataLockControlMode>",
            "<DataLockControlMode>AutomaticAndManaged</DataLockControlMode>",
            1,
        );
        assert_ne!(
            mixed_catalog, catalog,
            "the catalog must declare a lock mode"
        );
        unica_metadata_designer::format_contract::validate_metadata_8_3_27_enum_contract(
            &catalog, "test",
        )
        .unwrap();
        let rejected =
            unica_metadata_designer::format_contract::validate_metadata_8_3_27_enum_contract(
                &mixed_catalog,
                "test",
            )
            .unwrap_err();
        assert!(rejected.contains("DataLockControlMode"), "{rejected}");

        let document = Document::parse(&mixed_catalog).unwrap();
        let properties = document
            .descendants()
            .find(|node| node.has_tag_name((MD_NS, "Properties")))
            .unwrap();
        let mut reporter = MetaValidationReporter::new(30);
        meta_validate_check_property_values(&mut reporter, Some(properties));
        assert!(
            reporter
                .errors
                .iter()
                .any(|error| error.contains("DataLockControlMode")),
            "{:?}",
            reporter.errors
        );
    }

    fn context() -> WorkspaceContext {
        WorkspaceContext {
            cwd: PathBuf::from("/workspace"),
            workspace_root: PathBuf::from("/workspace"),
            cache_root: PathBuf::from("/workspace/.unica/cache"),
            workspace_epoch: 1,
        }
    }

    fn address(value: &str) -> MetadataAddress {
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, value).unwrap()
    }

    fn image(
        role: unica_metadata::validation_subject::MetadataResourceRole,
        xml: impl Into<Vec<u8>>,
    ) -> unica_metadata::validation_subject::MetadataResourceImage {
        unica_metadata::validation_subject::MetadataResourceImage {
            role,
            bytes: xml.into(),
        }
    }

    fn subject(
        target: &str,
        descriptor: Option<&str>,
        registration: &str,
        dependencies: &[(&str, &str)],
    ) -> unica_metadata::validation_subject::MetadataValidationSubject {
        let mut resources = Vec::new();
        if let Some(descriptor) = descriptor {
            resources.push(image(
                unica_metadata::validation_subject::MetadataResourceRole::Descriptor,
                descriptor.as_bytes().to_vec(),
            ));
        }
        resources.push(image(
            unica_metadata::validation_subject::MetadataResourceRole::Registration,
            registration.as_bytes().to_vec(),
        ));
        resources.extend(dependencies.iter().map(|(target, dependency)| {
            image(
                unica_metadata::validation_subject::MetadataResourceRole::Dependency {
                    target: address(target),
                },
                dependency.as_bytes().to_vec(),
            )
        }));
        unica_metadata::validation_subject::MetadataValidationSubject {
            target: address(target),
            resources,
            child_footprints: Vec::new(),
            registrar_evidence: Default::default(),
            subsystem_evidence: Default::default(),
        }
    }

    fn command_interface_rule_warnings(
        subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    ) -> Vec<String> {
        let document = Document::parse(
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses"><UseStandardCommands>true</UseStandardCommands><WriteMode>Independent</WriteMode></Properties>"#,
        )
        .unwrap();
        let mut reporter = MetaValidationReporter::new(30);
        let mut issues = 0;
        meta_validate_check_register_command_interface(
            &mut reporter,
            "InformationRegister",
            document.root_element(),
            Some(subject),
            "Ledger",
            &mut issues,
        );
        reporter.warnings
    }

    fn command_interface_subject(
        evidence: Option<unica_metadata::validation_subject::MetadataSubsystemEvidence>,
    ) -> unica_metadata::validation_subject::MetadataValidationSubject {
        unica_metadata::validation_subject::MetadataValidationSubject {
            target: address("InformationRegister.Ledger"),
            resources: Vec::new(),
            child_footprints: Vec::new(),
            registrar_evidence:
                unica_metadata::validation_subject::MetadataEvidenceAvailability::Complete,
            subsystem_evidence: evidence,
        }
    }

    #[test]
    fn child_descriptor_read_admission_preserves_structure_and_strict_publication() {
        let owner =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Report.Sales").unwrap();
        let body = "<Template><Properties><Name>Dcs</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></Template>";
        for version in [Some("2.20"), Some("2.19"), Some("2.21"), None] {
            let version_attr = version
                .map(|v| format!(" version=\"{v}\""))
                .unwrap_or_default();
            let xml = format!(
                "<MetaDataObject xmlns=\"{MD_CLASSES_NS}\"{version_attr}>{body}</MetaDataObject>"
            );
            let read =
                parse_child_profile_from_bytes(xml.as_bytes(), &owner, MetadataAdmission::Read)
                    .unwrap()
                    .unwrap();
            assert_eq!(read.0.as_str(), "Report.Sales.Template.Dcs");
            assert_eq!(
                read.1,
                unica_metadata::validation_subject::MetadataChildProfile::Template(
                    unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema
                )
            );
            assert_eq!(
                parse_child_profile_from_bytes(xml.as_bytes(), &owner, MetadataAdmission::Writable)
                    .is_ok(),
                version == Some("2.20")
            );
            for malformed in [
                xml.replace(MD_CLASSES_NS, "urn:foreign"),
                xml.replace(body, &format!("{body}{body}")),
                xml.replace("<Name>Dcs</Name>", "<Name>Dcs</Name><Name>Other</Name>"),
                xml.replace("<Template>", "<Template xmlns=\"urn:foreign\">"),
                xml.replace("<Properties>", "<Properties xmlns=\"urn:foreign\">"),
            ] {
                assert!(
                    parse_child_profile_from_bytes(
                        malformed.as_bytes(),
                        &owner,
                        MetadataAdmission::Read
                    )
                    .is_err(),
                    "{malformed}"
                );
            }
        }
    }

    #[test]
    fn command_interface_rule_distinguishes_absence_presence_and_uncollected_evidence() {
        let missing = command_interface_subject(Some(
            unica_metadata::validation_subject::MetadataSubsystemEvidence::Complete {
                functional_subsystems: Vec::new(),
                interface_subsystems: Vec::new(),
            },
        ));
        assert!(command_interface_rule_warnings(&missing)
            .iter()
            .any(|warning| warning.contains("reaches no command interface section")));

        let present = command_interface_subject(Some(
            unica_metadata::validation_subject::MetadataSubsystemEvidence::Complete {
                functional_subsystems: Vec::new(),
                interface_subsystems: vec![SubsystemAddress::parse("Sales.Registers").unwrap()],
            },
        ));
        assert!(!command_interface_rule_warnings(&present)
            .iter()
            .any(|warning| warning.contains("reaches no command interface section")));

        let uncollected = command_interface_subject(None);
        assert!(command_interface_rule_warnings(&uncollected).is_empty());
    }

    #[test]
    fn unavailable_subsystem_evidence_is_not_a_semantic_absence() {
        let diagnostic = MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "subsystem topology could not be proved",
        )
        .with_metadata_path(address("InformationRegister.Ledger"))
        .with_field("subsystemEvidence");
        let subject = command_interface_subject(Some(
            unica_metadata::validation_subject::MetadataSubsystemEvidence::Unavailable(vec![
                diagnostic,
            ]),
        ));

        assert!(command_interface_rule_warnings(&subject).is_empty());
        let result =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate_complete_read(&subject, &context());
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == MetaDiagnosticCode::ProviderUnavailable
                && diagnostic.field.as_deref() == Some("subsystemEvidence")
        }));
        assert!(!result.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("reaches no command interface section")));
    }

    #[test]
    fn generic_dependency_does_not_stand_in_for_subsystem_evidence() {
        let mut subject = command_interface_subject(Some(
            unica_metadata::validation_subject::MetadataSubsystemEvidence::Complete {
                functional_subsystems: Vec::new(),
                interface_subsystems: Vec::new(),
            },
        ));
        subject.resources.push(image(
            unica_metadata::validation_subject::MetadataResourceRole::Dependency {
                target: address("Subsystem.Main"),
            },
            format!(
                r#"<MetaDataObject xmlns="{MD_NS}"><Subsystem><Properties><Name>Main</Name></Properties></Subsystem></MetaDataObject>"#
            ),
        ));

        assert!(command_interface_rule_warnings(&subject)
            .iter()
            .any(|warning| warning.contains("reaches no command interface section")));
    }

    fn common_module(name: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<CommonModule uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>{name}</Name></Properties><ChildObjects/>
</CommonModule></MetaDataObject>"#
        )
    }

    fn scheduled_job(name: &str, method: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<ScheduledJob uuid="55555555-5555-4555-8555-555555555555">
<Properties><Name>{name}</Name><MethodName>{method}</MethodName></Properties><ChildObjects/>
</ScheduledJob></MetaDataObject>"#
        )
    }

    fn owner(registrations: &[(&str, &str)]) -> String {
        let registrations = registrations
            .iter()
            .map(|(kind, name)| format!("<{kind}>{name}</{kind}>"))
            .collect::<String>();
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<Configuration uuid="22222222-2222-4222-8222-222222222222">
<Properties><Name>Owner</Name></Properties><ChildObjects>{registrations}</ChildObjects>
</Configuration></MetaDataObject>"#
        )
    }

    fn dependency_with_reference(reference: Option<&str>) -> String {
        let content = reference
            .map(|reference| format!("<Content><Item>{reference}</Item></Content>"))
            .unwrap_or_default();
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<Subsystem uuid="33333333-3333-4333-8333-333333333333">
<Properties><Name>Area</Name>{content}</Properties><ChildObjects/>
</Subsystem></MetaDataObject>"#
        )
    }

    fn assert_internal_status(
        _name: &str,
        subject: &unica_metadata::validation_subject::MetadataValidationSubject,
        expected: MetaValidationStatus,
    ) -> unica_metadata::MetaValidationData {
        let context = context();
        let internal =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(subject, &context);
        assert_eq!(internal.status, expected);
        internal
    }

    #[test]
    fn ordinary_form_validator_rederives_profile_resource_kind_and_ordinal() {
        let descriptor = typed_child_xml("Form", "Main", None).replace(
            "</Properties>",
            "<FormType>Ordinary</FormType></Properties>",
        );
        let mut subject = closed_child_subject(
            "Form",
            "Main",
            None,
            &descriptor,
            vec![
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
            ],
            vec![(
                unica_metadata::validation_subject::MetadataChildResourceKind::OrdinaryFormContent,
                0,
            )],
        );
        subject.resources[0].bytes = typed_owner_descriptor(&["<Form>Main</Form>".into()]);
        subject.child_footprints[0].profile =
            unica_metadata::validation_subject::MetadataChildProfile::OrdinaryForm;
        subject.resources.last_mut().unwrap().bytes = vec![0xff, 0, 0x81];
        assert!(closed_child_diagnostics(&subject).is_empty());

        let mut forged = subject.clone();
        forged.child_footprints[0].profile =
            unica_metadata::validation_subject::MetadataChildProfile::Form;
        assert!(!closed_child_diagnostics(&forged).is_empty());

        let mut forged = subject.clone();
        let unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
            ordinal, ..
        } = &mut forged.resources.last_mut().unwrap().role
        else {
            panic!("fixture child resource")
        };
        *ordinal = 1;
        assert!(!closed_child_diagnostics(&forged).is_empty());

        let mut forged = subject.clone();
        let unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
            kind, ..
        } = &mut forged.resources.last_mut().unwrap().role
        else {
            panic!("fixture child resource")
        };
        *kind = unica_metadata::validation_subject::MetadataChildResourceKind::FormContent;
        forged.resources.last_mut().unwrap().bytes =
            br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#.to_vec();
        assert!(!closed_child_diagnostics(&forged).is_empty());

        let mut forged = subject.clone();
        forged
            .resources
            .push(forged.resources.last().unwrap().clone());
        assert!(!closed_child_diagnostics(&forged).is_empty());

        let mut forged = subject;
        forged.resources[1].bytes = typed_child_descriptor(&descriptor.replace(
            "<FormType>Ordinary</FormType>",
            "<FormType>Managed</FormType>",
        ));
        assert!(!closed_child_diagnostics(&forged).is_empty());
    }

    #[test]
    fn retained_form_content_rejects_wrong_namespace_and_export_version() {
        for (label, bytes) in [
            (
                "namespace",
                br#"<Form xmlns="urn:not-logform" version="2.20"/>"#.as_slice(),
            ),
            (
                "version",
                br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#.as_slice(),
            ),
            ("malformed", br#"<Form"#.as_slice()),
            (
                "root",
                br#"<NotForm xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#.as_slice(),
            ),
            (
                "encoding",
                br#"<?xml version="1.0" encoding="windows-1251"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#
                    .as_slice(),
            ),
        ] {
            let subject = unica_metadata::validation_subject::MetadataValidationSubject {
                target: address("Catalog.Editable"),
                resources: vec![image(
                    unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                        child: address("Catalog.Editable.Form.Main"),
                        kind: unica_metadata::validation_subject::MetadataChildResourceKind::FormContent,
                        ordinal: 0,
                    },
                    bytes,
                )],
                child_footprints: Vec::new(),
                registrar_evidence: Default::default(),
            subsystem_evidence: Default::default(),
            };

            let result = MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(unica_forms_designer::root::require_form_root)).validate(&subject, &context());

            assert_eq!(result.status, MetaValidationStatus::Failed, "{label}");
            assert_eq!(
                result.diagnostics[0].field.as_deref(),
                Some("resources[0].bytes"),
                "{label}"
            );
        }
    }

    fn template_subject(
        child: &str,
        resources: Vec<(
            unica_metadata::validation_subject::MetadataTemplateType,
            unica_metadata::validation_subject::MetadataTemplateResourcePart,
            Vec<u8>,
        )>,
    ) -> unica_metadata::validation_subject::MetadataValidationSubject {
        let child_address = address(child);
        let name = child_address.segments().last().unwrap().to_string();
        let template_type = resources.first().unwrap().0;
        let template_type_value = template_type.descriptor_value();
        let descriptor = typed_child_xml("Template", &name, Some(template_type_value));
        let directories = if template_type
            == unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument
        {
            vec![
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::HtmlPages,
            ]
        } else {
            vec![
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
            ]
        };
        let kinds = resources
            .iter()
            .enumerate()
            .map(|(ordinal, (template_type, part, _))| {
                (
                    unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                        template_type: *template_type,
                        part: *part,
                    },
                    ordinal,
                )
            })
            .collect();
        let mut subject = closed_child_subject(
            "Template",
            &name,
            Some(template_type_value),
            &descriptor,
            directories,
            kinds,
        );
        // Payload checks use the platform's by-name registration; legacy inline
        // diagnostics have their own canonical fixtures and descriptor guards.
        subject.resources[0].bytes =
            typed_owner_descriptor(&[format!("<Template>{name}</Template>")]);
        for (ordinal, (_, _, bytes)) in resources.into_iter().enumerate() {
            if let Some(resource) = subject.resources.iter_mut().find(|resource| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                        ordinal: candidate,
                        ..
                    } if candidate == ordinal
                )
            }) {
                resource.bytes = bytes;
            }
        }
        subject
    }

    fn typed_child_xml(kind: &str, name: &str, template_type: Option<&str>) -> String {
        let template_type = template_type
            .map(|value| format!("<TemplateType>{value}</TemplateType>"))
            .unwrap_or_default();
        format!(
            r#"<{kind} uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>{name}</Name>{template_type}</Properties><ChildObjects/></{kind}>"#
        )
    }

    fn typed_owner_descriptor(children: &[String]) -> Vec<u8> {
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><Catalog uuid="66666666-6666-4666-8666-666666666666"><Properties><Name>Editable</Name></Properties><ChildObjects>{}</ChildObjects></Catalog></MetaDataObject>"#,
            children.join("")
        )
        .into_bytes()
    }

    fn typed_owner_descriptor_with_structure(structure: &str) -> Vec<u8> {
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><Catalog uuid="66666666-6666-4666-8666-666666666666"><Properties><Name>Editable</Name></Properties>{structure}</Catalog></MetaDataObject>"#
        )
        .into_bytes()
    }

    fn typed_child_descriptor(child: &str) -> Vec<u8> {
        format!(r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">{child}</MetaDataObject>"#)
            .into_bytes()
    }

    fn closed_child_subject(
        kind: &str,
        name: &str,
        template_type: Option<&str>,
        descriptor_child: &str,
        directories: Vec<unica_metadata::validation_subject::MetadataChildDirectoryKind>,
        child_resources: Vec<(
            unica_metadata::validation_subject::MetadataChildResourceKind,
            usize,
        )>,
    ) -> unica_metadata::validation_subject::MetadataValidationSubject {
        let owner = address("Catalog.Editable");
        let expected_child = typed_child_xml(kind, name, template_type);
        let child = address(&format!("Catalog.Editable.{kind}.{name}"));
        let profile = match kind {
            "Form" => unica_metadata::validation_subject::MetadataChildProfile::Form,
            "Command" => unica_metadata::validation_subject::MetadataChildProfile::Command,
            "Template" => unica_metadata::validation_subject::MetadataChildProfile::Template(
                unica_metadata::validation_subject::MetadataTemplateType::from_descriptor_value(
                    template_type.unwrap(),
                )
                .unwrap(),
            ),
            _ => unreachable!(),
        };
        let descriptor_role = match kind {
            "Form" => unica_metadata::validation_subject::MetadataResourceRole::Form {
                owner: owner.clone(),
                name: name.to_string(),
            },
            "Template" => unica_metadata::validation_subject::MetadataResourceRole::Template {
                owner: owner.clone(),
                name: name.to_string(),
            },
            "Command" => unica_metadata::validation_subject::MetadataResourceRole::Command {
                owner,
                name: name.to_string(),
            },
            _ => unreachable!(),
        };
        let mut resources = vec![
            image(
                unica_metadata::validation_subject::MetadataResourceRole::Descriptor,
                typed_owner_descriptor(&[expected_child]),
            ),
            image(descriptor_role, typed_child_descriptor(descriptor_child)),
        ];
        resources.extend(child_resources.into_iter().map(|(kind, ordinal)| {
            image(
                unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                    child: child.clone(),
                    kind,
                    ordinal,
                },
                Vec::new(),
            )
        }));
        unica_metadata::validation_subject::MetadataValidationSubject {
            target: address("Catalog.Editable"),
            resources,
            child_footprints: vec![
                unica_metadata::validation_subject::MetadataChildFootprintEvidence {
                    child,
                    profile,
                    directories,
                    retained: Vec::new(),
                },
            ],
            registrar_evidence: Default::default(),
            subsystem_evidence: Default::default(),
        }
    }

    fn closed_child_diagnostics(
        subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    ) -> Vec<String> {
        let mut messages = subject
            .resources
            .iter()
            .filter_map(|resource| match resource.role {
                unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                    kind,
                    ..
                } => validate_child_resource(
                    unica_metadata_designer::validation::FormRootValidator(
                        unica_forms_designer::root::require_form_root,
                    ),
                    kind,
                    &resource.bytes,
                )
                .err(),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut diagnostics = Vec::new();
        validate_child_footprints(subject, &mut diagnostics, MetadataAdmission::Writable);
        messages.extend(diagnostics.into_iter().map(|diagnostic| diagnostic.message));
        messages
    }

    fn closed_validator_subject(
        kind: &str,
        template_type: Option<&str>,
        owner_entry: &str,
        descriptor_child: &str,
    ) -> unica_metadata::validation_subject::MetadataValidationSubject {
        let (directories, resources) = match kind {
            "Form" => (
                vec![
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                ],
                vec![(unica_metadata::validation_subject::MetadataChildResourceKind::FormContent, 0)],
            ),
            "Template" => (
                vec![
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                ],
                vec![(
                    unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                        template_type: unica_metadata::validation_subject::MetadataTemplateType::TextDocument,
                        part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    },
                    0,
                )],
            ),
            "Command" => (Vec::new(), Vec::new()),
            _ => unreachable!(),
        };
        let mut subject = closed_child_subject(
            kind,
            "Main",
            template_type,
            descriptor_child,
            directories,
            resources,
        );
        subject.resources[0].bytes = typed_owner_descriptor(&[owner_entry.to_string()]);
        for resource in &mut subject.resources {
            if let unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                kind,
                ..
            } = resource.role
            {
                resource.bytes = match kind {
                    unica_metadata::validation_subject::MetadataChildResourceKind::FormContent => br#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#.to_vec(),
                    unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent { .. } => b"text".to_vec(),
                    unica_metadata::validation_subject::MetadataChildResourceKind::Module
                    | unica_metadata::validation_subject::MetadataChildResourceKind::OrdinaryFormContent
                    | unica_metadata::validation_subject::MetadataChildResourceKind::Retained => unreachable!(),
                };
            }
        }
        subject.resources.push(image(
            unica_metadata::validation_subject::MetadataResourceRole::Registration,
            owner(&[("Catalog", "Editable")]),
        ));
        subject
    }

    fn assert_exact_closed_diagnostic(
        subject: &unica_metadata::validation_subject::MetadataValidationSubject,
        expected: &str,
    ) {
        let result =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(subject, &context());
        assert_eq!(result.status, MetaValidationStatus::Failed, "{result:?}");
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == expected)
            .unwrap_or_else(|| panic!("missing `{expected}` in {:?}", result.diagnostics));
        assert_eq!(diagnostic.code, MetaDiagnosticCode::ProviderUnavailable);
        assert_eq!(diagnostic.metadata_path, Some(subject.target.clone()));
        assert_eq!(diagnostic.field.as_deref(), Some("resources"));
        assert_eq!(diagnostic.operation_index, None);
        assert!(!diagnostic.message.contains("/workspace"));
    }

    fn closed_owner_subject(
        descriptor: &str,
    ) -> unica_metadata::validation_subject::MetadataValidationSubject {
        subject(
            "CommonModule.Service",
            Some(descriptor),
            &owner(&[("CommonModule", "Service")]),
            &[],
        )
    }

    #[test]
    fn closed_validator_rejects_foreign_properties_in_the_owner_descriptor() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><x:Properties xmlns:x="urn:foreign"><x:Name>Service</x:Name></x:Properties><ChildObjects/></CommonModule></MetaDataObject>"#
        );
        let subject = closed_owner_subject(&descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "owner descriptor contains Properties outside the MDClasses namespace: CommonModule.Service",
        );
    }

    #[test]
    fn closed_validator_rejects_foreign_name_in_exact_owner_properties() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><Properties><x:Name xmlns:x="urn:foreign">Service</x:Name></Properties><ChildObjects/></CommonModule></MetaDataObject>"#
        );
        let subject = closed_owner_subject(&descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "owner descriptor contains Name outside the MDClasses namespace: CommonModule.Service",
        );
    }

    #[test]
    fn closed_validator_rejects_mdclasses_and_foreign_owner_properties_duplicates() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Service</Name></Properties><x:Properties xmlns:x="urn:foreign"><x:Name>Service</x:Name></x:Properties><ChildObjects/></CommonModule></MetaDataObject>"#
        );
        let subject = closed_owner_subject(&descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "owner descriptor contains Properties outside the MDClasses namespace: CommonModule.Service",
        );
    }

    #[test]
    fn closed_validator_rejects_mdclasses_and_foreign_owner_name_duplicates() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Service</Name><x:Name xmlns:x="urn:foreign">Service</x:Name></Properties><ChildObjects/></CommonModule></MetaDataObject>"#
        );
        let subject = closed_owner_subject(&descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "owner descriptor contains Name outside the MDClasses namespace: CommonModule.Service",
        );
    }

    #[test]
    fn closed_validator_accepts_a_canonical_mdclasses_owner_identity() {
        let descriptor = common_module("Service");
        let subject = closed_owner_subject(&descriptor);

        let result =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(&subject, &context());

        assert_eq!(result.status, MetaValidationStatus::Passed, "{result:?}");
    }

    #[test]
    fn closed_validator_rejects_foreign_childobjects_in_the_final_owner_graph() {
        let child = typed_child_xml("Command", "Main", None);
        let owner_entry =
            format!(r#"<x:ChildObjects xmlns:x="urn:foreign">{child}</x:ChildObjects>"#);
        let descriptor = typed_child_xml("Command", "Main", None);
        let mut subject = closed_validator_subject(
            "Command",
            None,
            &typed_child_xml("Command", "Main", None),
            &descriptor,
        );
        subject.resources[0].bytes = typed_owner_descriptor_with_structure(&owner_entry);

        assert_exact_closed_diagnostic(
            &subject,
            "owner descriptor contains ChildObjects outside the MDClasses namespace: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_foreign_properties_in_each_child_descriptor_kind() {
        for (kind, template_type, foreign_properties) in [
            (
                "Form",
                None,
                r#"<x:Properties xmlns:x="urn:foreign"><x:Name>Main</x:Name></x:Properties>"#,
            ),
            (
                "Template",
                Some("TextDocument"),
                r#"<x:Properties xmlns:x="urn:foreign"><x:Name>Main</x:Name><x:TemplateType>TextDocument</x:TemplateType></x:Properties>"#,
            ),
            (
                "Command",
                None,
                r#"<x:Properties xmlns:x="urn:foreign"><x:Name>Main</x:Name></x:Properties>"#,
            ),
        ] {
            let owner_entry = typed_child_xml(kind, "Main", template_type);
            let descriptor = format!(
                r#"<{kind} uuid="77777777-7777-4777-8777-777777777777">{foreign_properties}<ChildObjects/></{kind}>"#
            );
            let subject = closed_validator_subject(kind, template_type, &owner_entry, &descriptor);

            assert_exact_closed_diagnostic(
                &subject,
                &format!(
                    "declared child descriptor is invalid: {kind} descriptor contains Properties outside the MDClasses namespace: Catalog.Editable"
                ),
            );
        }
    }

    #[test]
    fn closed_validator_rejects_foreign_name_under_exact_properties() {
        let owner_entry = typed_child_xml("Form", "Main", None);
        let descriptor = r#"<Form uuid="77777777-7777-4777-8777-777777777777"><Properties><x:Name xmlns:x="urn:foreign">Main</x:Name></Properties><ChildObjects/></Form>"#;
        let subject = closed_validator_subject("Form", None, &owner_entry, descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "declared child descriptor is invalid: Form descriptor contains Name outside the MDClasses namespace: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_foreign_templatetype_under_exact_template_properties() {
        let owner_entry = typed_child_xml("Template", "Main", Some("TextDocument"));
        let descriptor = r#"<Template uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name><x:TemplateType xmlns:x="urn:foreign">TextDocument</x:TemplateType></Properties><ChildObjects/></Template>"#;
        let subject =
            closed_validator_subject("Template", Some("TextDocument"), &owner_entry, descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "declared child descriptor is invalid: Template descriptor contains TemplateType outside the MDClasses namespace: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_foreign_childobjects_in_a_child_descriptor() {
        let owner_entry = typed_child_xml("Form", "Main", None);
        let descriptor = r#"<Form uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name></Properties><x:ChildObjects xmlns:x="urn:foreign"/></Form>"#;
        let subject = closed_validator_subject("Form", None, &owner_entry, descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "declared child descriptor is invalid: Form descriptor contains ChildObjects outside the MDClasses namespace: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_foreign_templatetype_in_an_inline_owner_entry() {
        let owner_entry = r#"<Template uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name><x:TemplateType xmlns:x="urn:foreign">TextDocument</x:TemplateType></Properties><ChildObjects/></Template>"#;
        let descriptor = typed_child_xml("Template", "Main", Some("TextDocument"));
        let subject =
            closed_validator_subject("Template", Some("TextDocument"), owner_entry, &descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "Template descriptor contains TemplateType outside the MDClasses namespace: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_an_additional_foreign_root_artifact() {
        let owner_entry = typed_child_xml("Command", "Main", None);
        let valid = typed_child_xml("Command", "Main", None);
        let descriptor = format!(r#"{valid}<x:Ignored xmlns:x="urn:foreign"/>"#);
        let subject = closed_validator_subject("Command", None, &owner_entry, &descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "declared child descriptor is invalid: metadata descriptor must contain exactly one element artifact: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_mdclasses_and_foreign_structural_duplicates() {
        let cases = [
            (
                "ChildObjects",
                "Command",
                None,
                format!(
                    r#"<ChildObjects>{}</ChildObjects><x:ChildObjects xmlns:x="urn:foreign"/>"#,
                    typed_child_xml("Command", "Main", None)
                ),
                typed_child_xml("Command", "Main", None),
                "owner descriptor contains ChildObjects outside the MDClasses namespace: Catalog.Editable".to_string(),
            ),
            (
                "Properties",
                "Command",
                None,
                typed_child_xml("Command", "Main", None),
                r#"<Command uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name></Properties><x:Properties xmlns:x="urn:foreign"><x:Name>Main</x:Name></x:Properties><ChildObjects/></Command>"#.to_string(),
                "declared child descriptor is invalid: Command descriptor contains Properties outside the MDClasses namespace: Catalog.Editable".to_string(),
            ),
            (
                "Name",
                "Form",
                None,
                typed_child_xml("Form", "Main", None),
                r#"<Form uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name><x:Name xmlns:x="urn:foreign">Main</x:Name></Properties><ChildObjects/></Form>"#.to_string(),
                "declared child descriptor is invalid: Form descriptor contains Name outside the MDClasses namespace: Catalog.Editable".to_string(),
            ),
            (
                "TemplateType",
                "Template",
                Some("TextDocument"),
                typed_child_xml("Template", "Main", Some("TextDocument")),
                r#"<Template uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name><TemplateType>TextDocument</TemplateType><x:TemplateType xmlns:x="urn:foreign">TextDocument</x:TemplateType></Properties><ChildObjects/></Template>"#.to_string(),
                "declared child descriptor is invalid: Template descriptor contains TemplateType outside the MDClasses namespace: Catalog.Editable".to_string(),
            ),
        ];
        for (label, kind, template_type, owner_entry, descriptor, expected) in cases {
            let mut subject =
                closed_validator_subject(kind, template_type, &owner_entry, &descriptor);
            if label == "ChildObjects" {
                subject.resources[0].bytes = typed_owner_descriptor_with_structure(&owner_entry);
            }
            assert_exact_closed_diagnostic(&subject, &expected);
            assert!(!expected.contains('/'), "{label}");
        }
    }

    #[test]
    fn readable_child_graph_does_not_authorize_materializing_unsupported_descriptors() {
        for kind in ["Template", "Form"] {
            let template_type = (kind == "Template").then_some("TextDocument");
            let child = typed_child_xml(kind, "Main", template_type);
            for version in [Some("2.21"), Some("2.19"), None] {
                // Either the existing owner or an adopted child's descriptor can
                // carry the incompatible profile. Reading is not a write grant.
                for resource_index in [0, 1] {
                    let mut subject = closed_validator_subject(
                        kind,
                        template_type,
                        &format!("<{kind}>Main</{kind}>"),
                        &child,
                    );
                    let original =
                        String::from_utf8(subject.resources[resource_index].bytes.clone()).unwrap();
                    subject.resources[resource_index].bytes = original
                        .replace(
                            "version=\"2.20\"",
                            &version
                                .map(|v| format!("version=\"{v}\""))
                                .unwrap_or_default(),
                        )
                        .into_bytes();
                    let mut read = Vec::new();
                    validate_child_footprints(&subject, &mut read, MetadataAdmission::Read);
                    assert!(read.is_empty(), "{kind} {version:?}: {read:?}");
                    let mut publication = Vec::new();
                    validate_child_footprints(
                        &subject,
                        &mut publication,
                        MetadataAdmission::Writable,
                    );
                    assert!(
                        !publication.is_empty(),
                        "{kind} {version:?}: read proof authorized publication"
                    );
                }
            }
        }
    }

    #[test]
    fn closed_child_graph_accepts_real_text_refs_and_typed_inline_owner_entries() {
        for (label, owner_entry) in [
            ("real-ref", "<Command>Main</Command>".to_string()),
            ("typed-inline", typed_child_xml("Command", "Main", None)),
        ] {
            let descriptor = typed_child_xml("Command", "Main", None);
            let subject = closed_validator_subject("Command", None, &owner_entry, &descriptor);

            let mut diagnostics = Vec::new();
            validate_child_footprints(&subject, &mut diagnostics, MetadataAdmission::Writable);

            assert!(diagnostics.is_empty(), "{label}: {diagnostics:?}");
        }
    }

    #[test]
    fn final_child_graph_rejects_wholly_omitted_evidence_for_every_profile() {
        for (label, mut subject) in [
            (
                "form",
                closed_child_subject(
                    "Form",
                    "Main",
                    None,
                    &typed_child_xml("Form", "Main", None),
                    vec![
                        unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                        unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                    ],
                    vec![(unica_metadata::validation_subject::MetadataChildResourceKind::FormContent, 0)],
                ),
            ),
            (
                "template",
                closed_child_subject(
                    "Template",
                    "Main",
                    Some("TextDocument"),
                    &typed_child_xml("Template", "Main", Some("TextDocument")),
                    vec![
                        unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                        unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                    ],
                    vec![(
                        unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                            template_type: unica_metadata::validation_subject::MetadataTemplateType::TextDocument,
                            part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                        },
                        0,
                    )],
                ),
            ),
            (
                "command",
                closed_child_subject(
                    "Command",
                    "Main",
                    None,
                    &typed_child_xml("Command", "Main", None),
                    Vec::new(),
                    Vec::new(),
                ),
            ),
        ] {
            subject.child_footprints.clear();
            let mut diagnostics = Vec::new();

            validate_child_footprints(&subject, &mut diagnostics, MetadataAdmission::Writable);

            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("footprint evidence")),
                "{label}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn child_descriptor_bytes_reject_forged_role_kind_and_name_for_every_profile() {
        let cases = [
            (
                "form-name",
                "Form",
                None,
                typed_child_xml("Form", "Other", None),
                vec![(unica_metadata::validation_subject::MetadataChildResourceKind::FormContent, 0)],
            ),
            (
                "form-kind",
                "Form",
                None,
                typed_child_xml("Template", "Main", Some("TextDocument")),
                vec![(unica_metadata::validation_subject::MetadataChildResourceKind::FormContent, 0)],
            ),
            (
                "template-name",
                "Template",
                Some("TextDocument"),
                typed_child_xml("Template", "Other", Some("TextDocument")),
                vec![(
                    unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                        template_type: unica_metadata::validation_subject::MetadataTemplateType::TextDocument,
                        part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    },
                    0,
                )],
            ),
            (
                "template-kind",
                "Template",
                Some("TextDocument"),
                typed_child_xml("Form", "Main", None),
                vec![(
                    unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                        template_type: unica_metadata::validation_subject::MetadataTemplateType::TextDocument,
                        part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    },
                    0,
                )],
            ),
            (
                "template-type",
                "Template",
                Some("TextDocument"),
                typed_child_xml("Template", "Main", Some("HTMLDocument")),
                vec![(
                    unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                        template_type: unica_metadata::validation_subject::MetadataTemplateType::TextDocument,
                        part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    },
                    0,
                )],
            ),
            (
                "command-name",
                "Command",
                None,
                typed_child_xml("Command", "Other", None),
                Vec::new(),
            ),
            (
                "command-kind",
                "Command",
                None,
                typed_child_xml("Form", "Main", None),
                Vec::new(),
            ),
        ];
        for (label, kind, template_type, forged_descriptor, resources) in cases {
            let directories = if kind == "Command" {
                Vec::new()
            } else {
                vec![
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                ]
            };
            let subject = closed_child_subject(
                kind,
                "Main",
                template_type,
                &forged_descriptor,
                directories,
                resources,
            );
            let mut diagnostics = Vec::new();

            validate_child_footprints(&subject, &mut diagnostics, MetadataAdmission::Writable);

            // A forged name or kind moves the descriptor to another child
            // identity, so the expected child is left with none at all.
            let expected_message = if label == "template-type" {
                "footprint evidence"
            } else {
                "no descriptor among the read resources"
            };
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(expected_message)),
                "{label}: {diagnostics:?}"
            );
            if label != "template-type" {
                assert!(
                    diagnostics
                        .iter()
                        .all(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error),
                    "a descriptor for another identity/kind must not produce a legacy warning: {label}: {diagnostics:?}"
                );
            }
        }

        let form = typed_child_xml("Form", "Main", None);
        for (label, descriptor) in [
            (
                "root-namespace",
                String::from_utf8(typed_child_descriptor(&form))
                    .unwrap()
                    .replace(MD_CLASSES_NS, "urn:foreign"),
            ),
            (
                "root-version",
                String::from_utf8(typed_child_descriptor(&form))
                    .unwrap()
                    .replace("version=\"2.20\"", "version=\"2.19\""),
            ),
            (
                "artifact-namespace",
                format!(
                    r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><x:Form xmlns:x="urn:foreign"><x:Properties><x:Name>Main</x:Name></x:Properties><x:ChildObjects/></x:Form></MetaDataObject>"#
                ),
            ),
        ] {
            let subject = closed_child_subject(
                "Form",
                "Main",
                None,
                &form,
                vec![
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                ],
                vec![(
                    unica_metadata::validation_subject::MetadataChildResourceKind::FormContent,
                    0,
                )],
            );
            let mut subject = subject;
            subject.resources[1].bytes = descriptor.into_bytes();
            let mut diagnostics = Vec::new();

            validate_child_footprints(&subject, &mut diagnostics, MetadataAdmission::Writable);

            assert!(
                diagnostics.iter().any(|diagnostic| diagnostic
                    .message
                    .contains("no descriptor among the read resources")),
                "{label}: {diagnostics:?}"
            );
            assert!(
                diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error),
                "an invalid descriptor namespace/profile must not produce a legacy warning: {label}: {diagnostics:?}"
            );
        }
    }

    /// Issue #360 asked for this: the three ways a child can lose its
    /// descriptor used to share one message, so the diagnostic never said which
    /// of them happened.
    #[test]
    fn each_missing_child_descriptor_cause_names_itself() {
        let form = typed_child_xml("Form", "Main", None);
        let subject = || {
            closed_child_subject(
                "Form",
                "Main",
                None,
                &form,
                vec![
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                ],
                vec![(
                    unica_metadata::validation_subject::MetadataChildResourceKind::FormContent,
                    0,
                )],
            )
        };

        let mut absent = subject();
        absent.resources.remove(1);
        let mut duplicated = subject();
        duplicated.resources.push(duplicated.resources[1].clone());
        let mut misfiled = subject();
        misfiled.resources[1].role =
            unica_metadata::validation_subject::MetadataResourceRole::Form {
                owner: address("Catalog.Editable"),
                name: "Other".to_string(),
            };

        for (label, subject, expected) in [
            (
                "absent",
                absent,
                "final child has no descriptor among the read resources",
            ),
            (
                "duplicated",
                duplicated,
                "final child is backed by more than one descriptor",
            ),
            (
                "misfiled",
                misfiled,
                "final child descriptor is filed under a resource role naming another child",
            ),
        ] {
            let mut diagnostics = Vec::new();

            validate_child_footprints(&subject, &mut diagnostics, MetadataAdmission::Writable);

            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.starts_with(expected)),
                "{label}: {diagnostics:?}"
            );
            assert!(
                diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error),
                "missing, duplicate or misfiled descriptor evidence must not produce a legacy warning: {label}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn child_footprint_sets_are_order_independent_but_ordinals_are_canonical() {
        let html = typed_child_xml("Template", "Main", Some("HTMLDocument"));
        let primary =
            unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                template_type:
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
            };
        let page = unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
            template_type: unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
            part: unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
        };
        let set_html_descriptor =
            |subject: &mut unica_metadata::validation_subject::MetadataValidationSubject| {
                subject
                .resources
                .iter_mut()
                .find(|resource| {
                    matches!(
                        resource.role,
                        unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                            kind: unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                                template_type: unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                                part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                            },
                            ..
                        }
                    )
                })
                .unwrap()
                .bytes = br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#
                .to_vec();
            };
        let mut reordered = closed_child_subject(
            "Template",
            "Main",
            Some("HTMLDocument"),
            &html,
            vec![
                unica_metadata::validation_subject::MetadataChildDirectoryKind::HtmlPages,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
            ],
            vec![(page, 1), (primary, 0)],
        );
        reordered.resources[0].bytes =
            typed_owner_descriptor(&["<Template>Main</Template>".into()]);
        set_html_descriptor(&mut reordered);
        reordered.resources.reverse();
        let mut diagnostics = Vec::new();
        validate_child_footprints(&reordered, &mut diagnostics, MetadataAdmission::Writable);
        assert!(diagnostics.is_empty(), "reordered: {diagnostics:?}");

        for (label, resources) in [
            ("duplicate", vec![(primary, 0), (page, 0)]),
            ("gap", vec![(primary, 0), (page, 2)]),
            ("swapped", vec![(primary, 1), (page, 0)]),
        ] {
            let mut subject = closed_child_subject(
                "Template",
                "Main",
                Some("HTMLDocument"),
                &html,
                vec![
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                    unica_metadata::validation_subject::MetadataChildDirectoryKind::HtmlPages,
                ],
                resources,
            );
            set_html_descriptor(&mut subject);
            let mut diagnostics = Vec::new();
            validate_child_footprints(&subject, &mut diagnostics, MetadataAdmission::Writable);
            assert!(
                diagnostics.iter().any(|diagnostic| diagnostic
                    .message
                    .contains("canonical resource and ordinal set")),
                "{label}: {diagnostics:?}"
            );
        }

        let mut duplicate_directory = closed_child_subject(
            "Template",
            "Main",
            Some("HTMLDocument"),
            &html,
            vec![
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::HtmlPages,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
            ],
            vec![(primary, 0), (page, 1)],
        );
        duplicate_directory.child_footprints.reverse();
        let mut diagnostics = Vec::new();
        validate_child_footprints(
            &duplicate_directory,
            &mut diagnostics,
            MetadataAdmission::Writable,
        );
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("directory set")));

        let form = typed_child_xml("Form", "Main", None);
        let subject = closed_child_subject(
            "Form",
            "Main",
            None,
            &form,
            vec![
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
            ],
            vec![
                (
                    unica_metadata::validation_subject::MetadataChildResourceKind::Module,
                    0,
                ),
                (
                    unica_metadata::validation_subject::MetadataChildResourceKind::FormContent,
                    1,
                ),
            ],
        );
        let mut diagnostics = Vec::new();
        validate_child_footprints(&subject, &mut diagnostics, MetadataAdmission::Writable);
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("canonical resource and ordinal set")));
    }

    #[test]
    fn complete_multi_child_graph_is_permutation_safe_and_rejects_duplicate_or_orphan_groups() {
        let form_xml = typed_child_xml("Form", "Entry", None);
        let template_xml = typed_child_xml("Template", "Print", Some("TextDocument"));
        let command_xml = typed_child_xml("Command", "Run", None);
        let mut form = closed_child_subject(
            "Form",
            "Entry",
            None,
            &form_xml,
            vec![
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
            ],
            vec![(
                unica_metadata::validation_subject::MetadataChildResourceKind::FormContent,
                0,
            )],
        );
        let template = closed_child_subject(
            "Template",
            "Print",
            Some("TextDocument"),
            &template_xml,
            vec![
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension,
                unica_metadata::validation_subject::MetadataChildDirectoryKind::Root,
            ],
            vec![(
                unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                    template_type:
                        unica_metadata::validation_subject::MetadataTemplateType::TextDocument,
                    part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                },
                0,
            )],
        );
        let command =
            closed_child_subject("Command", "Run", None, &command_xml, Vec::new(), Vec::new());
        form.resources[0].bytes = typed_owner_descriptor(&[
            "<Command>Run</Command>".to_string(),
            "<Form>Entry</Form>".to_string(),
            "<Template>Print</Template>".to_string(),
        ]);
        form.resources
            .extend(template.resources.into_iter().skip(1));
        form.resources.extend(command.resources.into_iter().skip(1));
        form.child_footprints.extend(template.child_footprints);
        form.child_footprints.extend(command.child_footprints);
        form.resources.reverse();
        form.child_footprints.reverse();

        let mut exact = Vec::new();
        validate_child_footprints(&form, &mut exact, MetadataAdmission::Writable);
        assert!(exact.is_empty(), "exact permutation: {exact:?}");

        let mut duplicate_descriptor = form.clone();
        let descriptor = duplicate_descriptor
            .resources
            .iter()
            .find(|resource| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::Form { .. }
                )
            })
            .unwrap()
            .clone();
        duplicate_descriptor.resources.push(descriptor);
        let mut diagnostics = Vec::new();
        validate_child_footprints(
            &duplicate_descriptor,
            &mut diagnostics,
            MetadataAdmission::Writable,
        );
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("backed by more than one descriptor")));

        let mut orphan_descriptor = form.clone();
        let orphan_xml = typed_child_xml("Form", "Orphan", None);
        orphan_descriptor.resources.push(image(
            unica_metadata::validation_subject::MetadataResourceRole::Form {
                owner: address("Catalog.Editable"),
                name: "Orphan".to_string(),
            },
            typed_child_descriptor(&orphan_xml),
        ));
        let mut diagnostics = Vec::new();
        validate_child_footprints(
            &orphan_descriptor,
            &mut diagnostics,
            MetadataAdmission::Writable,
        );
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("descriptor is orphaned")));

        let mut duplicate_evidence = form.clone();
        duplicate_evidence
            .child_footprints
            .push(duplicate_evidence.child_footprints[0].clone());
        let mut diagnostics = Vec::new();
        validate_child_footprints(
            &duplicate_evidence,
            &mut diagnostics,
            MetadataAdmission::Writable,
        );
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("exactly one matching footprint evidence")));

        let mut orphan_evidence = form.clone();
        orphan_evidence.child_footprints.push(
            unica_metadata::validation_subject::MetadataChildFootprintEvidence {
                child: address("Catalog.Editable.Command.Orphan"),
                profile: unica_metadata::validation_subject::MetadataChildProfile::Command,
                directories: Vec::new(),
                retained: Vec::new(),
            },
        );
        let mut diagnostics = Vec::new();
        validate_child_footprints(
            &orphan_evidence,
            &mut diagnostics,
            MetadataAdmission::Writable,
        );
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("evidence is orphaned")));

        let mut duplicate_resource = form.clone();
        let resource = duplicate_resource
            .resources
            .iter()
            .find(|resource| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                        kind: unica_metadata::validation_subject::MetadataChildResourceKind::FormContent,
                        ..
                    }
                )
            })
            .unwrap()
            .clone();
        duplicate_resource.resources.push(resource);
        let mut diagnostics = Vec::new();
        validate_child_footprints(
            &duplicate_resource,
            &mut diagnostics,
            MetadataAdmission::Writable,
        );
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("canonical resource and ordinal set")));

        let mut mislabeled_resource = form.clone();
        let resource = mislabeled_resource
            .resources
            .iter_mut()
            .find(|resource| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                        kind: unica_metadata::validation_subject::MetadataChildResourceKind::FormContent,
                        ..
                    }
                )
            })
            .unwrap();
        let unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
            child, ..
        } = &mut resource.role
        else {
            unreachable!()
        };
        *child = address("Catalog.Editable.Form.Orphan");
        let mut diagnostics = Vec::new();
        validate_child_footprints(
            &mislabeled_resource,
            &mut diagnostics,
            MetadataAdmission::Writable,
        );
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("canonical resource and ordinal set")
        }));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("resource is orphaned")));

        let mut duplicate_owner_child = form.clone();
        duplicate_owner_child
            .resources
            .iter_mut()
            .find(|resource| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::Descriptor
                )
            })
            .unwrap()
            .bytes = typed_owner_descriptor(&[
            typed_child_xml("Form", "Entry", None),
            typed_child_xml("Form", "Entry", None),
            typed_child_xml("Template", "Print", Some("TextDocument")),
            typed_child_xml("Command", "Run", None),
        ]);
        let mut diagnostics = Vec::new();
        validate_child_footprints(
            &duplicate_owner_child,
            &mut diagnostics,
            MetadataAdmission::Writable,
        );
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate child identity")));
    }

    #[test]
    fn template_payload_profiles_accept_representative_xml_text_and_binary_images() {
        let cases = [
            template_subject(
                "Catalog.Editable.Template.Spreadsheet",
                vec![(
                    unica_metadata::validation_subject::MetadataTemplateType::SpreadsheetDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    unica_templates_designer::mxl_root::empty_spreadsheet_document_xml().into_bytes(),
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Schema",
                vec![(
                    unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    br#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#
                        .to_vec(),
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Text",
                vec![(
                    unica_metadata::validation_subject::MetadataTemplateType::TextDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    "Текстовый макет".as_bytes().to_vec(),
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Binary",
                vec![(
                    unica_metadata::validation_subject::MetadataTemplateType::BinaryData,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    vec![0xff, 0x00, 0x80],
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Html",
                vec![
                    (
                        unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                        unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                        br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#.to_vec(),
                    ),
                    (
                        unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                        unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                        br#"<html><head><meta charset="utf-8"/></head><body/></html>"#.to_vec(),
                    ),
                ],
            ),
        ];

        for subject in cases {
            let diagnostics = closed_child_diagnostics(&subject);
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
        }
    }

    #[test]
    fn non_binary_template_profiles_reject_binary_or_wrong_xml_images() {
        let cases = [
            template_subject(
                "Catalog.Editable.Template.Schema",
                vec![(
                    unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    vec![0xff, 0x00, 0x80],
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Text",
                vec![(
                    unica_metadata::validation_subject::MetadataTemplateType::TextDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    vec![0xff, 0x00, 0x80],
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Spreadsheet",
                vec![(
                    unica_metadata::validation_subject::MetadataTemplateType::SpreadsheetDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    br#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#
                        .to_vec(),
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.VersionedSchema",
                vec![(
                    unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    br#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" version="2.20"/>"#
                        .to_vec(),
                )],
            ),
        ];

        for subject in cases {
            let diagnostics = closed_child_diagnostics(&subject);
            assert!(!diagnostics.is_empty());
        }
    }

    #[test]
    fn html_template_requires_both_its_descriptor_and_language_page() {
        for (label, resources) in [
            (
                "missing page",
                vec![(
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#
                        .to_vec(),
                )],
            ),
            (
                "missing descriptor",
                vec![(
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                    br#"<html><body/></html>"#.to_vec(),
                )],
            ),
        ] {
            let subject = template_subject("Catalog.Editable.Template.Html", resources);

            let diagnostics = closed_child_diagnostics(&subject);

            assert!(!diagnostics.is_empty(), "{label}");
        }
    }

    #[test]
    fn html_template_accepts_every_declared_language_page() {
        let subject = template_subject(
            "Catalog.Editable.Template.Html",
            vec![
                (
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page><Page>en</Page></Help>"#.to_vec(),
                ),
                (
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                    br#"<html><body>ru</body></html>"#.to_vec(),
                ),
                (
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                    br#"<html><body>en</body></html>"#.to_vec(),
                ),
            ],
        );

        let diagnostics = closed_child_diagnostics(&subject);

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn html_template_page_accepts_registered_utf8_html_without_xml_grammar() {
        let page = concat!(
            "<!DOCTYPE HTML PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\">\n",
            "<html><head><title>Справка&nbsp;1С</title></head>",
            "<body><p>Обычная HTML-страница<br></body></html>"
        );

        assert_eq!(
            validate_template_resource(
                unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                page.as_bytes(),
            ),
            Ok(())
        );
    }

    #[test]
    fn html_template_page_rejects_non_utf8_bytes() {
        let error = validate_template_resource(
            unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
            unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
            b"<html>\xff</html>",
        )
        .unwrap_err();

        assert!(error.contains("not UTF-8"), "{error}");
    }

    #[test]
    fn html_template_pages_must_match_registered_language_codes() {
        let subject = template_subject(
            "Catalog.Editable.Template.Html",
            vec![
                (
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page><Page>en</Page></Help>"#.to_vec(),
                ),
                (
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                    br#"<html><body>ru</body></html>"#.to_vec(),
                ),
                (
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                    br#"<html><body>en</body></html>"#.to_vec(),
                ),
            ],
        );
        let registered_languages = vec!["Русский".to_string(), "English".to_string()];
        let language_images = BTreeMap::from([
            ("Русский".to_string(), "ru".to_string()),
            ("English".to_string(), "en".to_string()),
        ]);
        let mut diagnostics = Vec::new();

        validate_html_template_page_registrations(
            &subject,
            &registered_languages,
            &language_images,
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let language_images = BTreeMap::from([
            ("Русский".to_string(), "ru".to_string()),
            ("English".to_string(), "de".to_string()),
        ]);
        validate_html_template_page_registrations(
            &subject,
            &registered_languages,
            &language_images,
            &mut diagnostics,
        );
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("`en` is not registered")));
    }

    #[test]
    fn child_footprint_evidence_is_independently_checked_against_resources_and_directories() {
        let mut subject = template_subject(
            "Catalog.Editable.Template.Html",
            vec![
                (
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#
                        .to_vec(),
                ),
                (
                    unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                    br#"<html><body/></html>"#.to_vec(),
                ),
            ],
        );
        subject.child_footprints[0].directories.retain(|directory| {
            *directory != unica_metadata::validation_subject::MetadataChildDirectoryKind::HtmlPages
        });

        let mut wrong_directories = Vec::new();
        validate_child_footprints(
            &subject,
            &mut wrong_directories,
            MetadataAdmission::Writable,
        );
        assert!(wrong_directories
            .iter()
            .any(|diagnostic| diagnostic.message.contains("directory set")));

        subject.child_footprints[0]
            .directories
            .push(unica_metadata::validation_subject::MetadataChildDirectoryKind::HtmlPages);
        subject.resources.pop();
        let mut missing_part = Vec::new();
        validate_child_footprints(&subject, &mut missing_part, MetadataAdmission::Writable);
        assert!(missing_part
            .iter()
            .any(|diagnostic| diagnostic.message.contains("canonical resource")));

        subject.resources.push(image(
            unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                child: address("Catalog.Editable.Template.Html"),
                kind: unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                    template_type: unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                    part: unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                },
                ordinal: 1,
            },
            br#"<html><body/></html>"#.to_vec(),
        ));
        let forged = typed_child_xml("Template", "Html", Some("SpreadsheetDocument"));
        subject
            .resources
            .iter_mut()
            .find(|resource| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::Template { .. }
                )
            })
            .unwrap()
            .bytes = typed_child_descriptor(&forged);
        let mut forged_profile = Vec::new();
        validate_child_footprints(&subject, &mut forged_profile, MetadataAdmission::Writable);
        assert!(forged_profile
            .iter()
            .any(|diagnostic| diagnostic.message.contains("footprint evidence")));

        let exact = typed_child_xml("Template", "Html", Some("HTMLDocument"));
        subject
            .resources
            .iter_mut()
            .find(|resource| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::Template { .. }
                )
            })
            .unwrap()
            .bytes = typed_child_descriptor(&exact);
        let mut exact = Vec::new();
        validate_child_footprints(&subject, &mut exact, MetadataAdmission::Writable);
        assert!(exact.is_empty(), "{exact:?}");
    }

    #[test]
    fn legacy_metadata_warnings_use_a_warning_code_and_keep_error_verdicts() {
        let registration = owner(&[("CommonModule", "Service")]);
        let no_context = common_module("Service").replace(
            "</Properties>",
            "<Server>false</Server><ClientManagedApplication>false</ClientManagedApplication><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ExternalConnection>false</ExternalConnection><ServerCall>false</ServerCall><Global>false</Global><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties>",
        );
        for (xml, expected) in [
            (no_context.clone(), MetaValidationStatus::Passed),
            (
                no_context.replace("DontUse", "InvalidReuse"),
                MetaValidationStatus::Failed,
            ),
        ] {
            let proof = subject("CommonModule.Service", Some(&xml), &registration, &[]);
            let result = assert_internal_status("context-warning", &proof, expected);
            let warning = result
                .diagnostics
                .iter()
                .find(|item| item.message == "10. CommonModule: no execution context enabled")
                .expect("real validator must report the disabled execution contexts");
            assert_eq!(warning.severity, MetaDiagnosticSeverity::Warning);
            assert_eq!(
                serde_json::to_value(warning.code).unwrap(),
                "validation_warning"
            );
            assert_eq!(warning.metadata_path, Some(address("CommonModule.Service")));
            assert_eq!(warning.field.as_deref(), Some("properties"));
            assert_eq!(warning.language, None);
            let errors = result
                .diagnostics
                .iter()
                .filter(|item| {
                    item.severity == MetaDiagnosticSeverity::Error
                        && item.code == MetaDiagnosticCode::ValidationFailed
                })
                .collect::<Vec<_>>();
            if expected == MetaValidationStatus::Failed {
                assert!(errors
                    .iter()
                    .any(|item| item.message.contains("InvalidReuse")));
            } else {
                assert!(errors.is_empty());
                assert!(result
                    .diagnostics
                    .iter()
                    .all(|item| item.code != MetaDiagnosticCode::ValidationFailed));
            }
        }
    }

    #[test]
    fn internal_valid_and_semantically_invalid_images_match_legacy_classification() {
        let registration = owner(&[("CommonModule", "Service")]);
        let valid_descriptor = common_module("Service");
        let valid = subject(
            "CommonModule.Service",
            Some(&valid_descriptor),
            &registration,
            &[],
        );
        assert_internal_status("valid", &valid, MetaValidationStatus::Passed);

        let invalid_descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<CommonModule uuid="11111111-1111-4111-8111-111111111111">
<Properties/><ChildObjects/>
</CommonModule></MetaDataObject>"#
        );
        let invalid = subject(
            "CommonModule.Service",
            Some(&invalid_descriptor),
            &registration,
            &[],
        );
        let result =
            assert_internal_status("semantic-invalid", &invalid, MetaValidationStatus::Failed);
        assert_eq!(
            result.diagnostics[0].code,
            MetaDiagnosticCode::ValidationFailed
        );
        assert_eq!(
            result.diagnostics[0].metadata_path,
            Some(address("CommonModule.Service"))
        );
        assert_eq!(result.diagnostics[0].operation_index, None);
        assert!(result.diagnostics[0].field.is_some());
    }

    #[test]
    fn internal_malformed_xml_is_a_hard_failure_matching_legacy_classification() {
        let registration = owner(&[("CommonModule", "Service")]);
        let malformed = subject(
            "CommonModule.Service",
            Some("<MetaDataObject"),
            &registration,
            &[],
        );

        let result = assert_internal_status(
            "malformed-descriptor",
            &malformed,
            MetaValidationStatus::Failed,
        );

        assert_eq!(
            result.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(
            result.diagnostics[0].field.as_deref(),
            Some("resources[0].bytes")
        );
    }

    #[test]
    fn internal_owner_registration_failure_matches_legacy_classification() {
        let descriptor = common_module("Service");
        let registration = owner(&[("CommonModule", "Other")]);
        let invalid = subject(
            "CommonModule.Service",
            Some(&descriptor),
            &registration,
            &[],
        );

        let result = assert_internal_status(
            "missing-registration",
            &invalid,
            MetaValidationStatus::Failed,
        );

        assert_eq!(
            result.diagnostics[0].code,
            MetaDiagnosticCode::ValidationFailed
        );
        assert_eq!(
            result.diagnostics[0].field.as_deref(),
            Some("resources[1].bytes")
        );
        assert!(result.diagnostics[0].message.contains("not registered"));
    }

    #[test]
    fn internal_nonexternal_descriptor_requires_owner_image() {
        let descriptor = common_module("Service");
        let incomplete = unica_metadata::validation_subject::MetadataValidationSubject {
            target: address("CommonModule.Service"),
            resources: vec![image(
                unica_metadata::validation_subject::MetadataResourceRole::Descriptor,
                descriptor.into_bytes(),
            )],
            child_footprints: Vec::new(),
            registrar_evidence: Default::default(),
            subsystem_evidence: Default::default(),
        };

        let result =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(&incomplete, &context());

        assert_eq!(result.status, MetaValidationStatus::Failed);
        assert_eq!(
            result.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(result.diagnostics[0].field.as_deref(), Some("resources"));
    }

    #[test]
    fn internal_duplicate_child_failure_matches_legacy_classification() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<HTTPService uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>Api</Name></Properties><ChildObjects>
<URLTemplate><Properties><Name>route</Name><Template>/one</Template></Properties><ChildObjects/></URLTemplate>
<URLTemplate><Properties><Name>route</Name><Template>/two</Template></Properties><ChildObjects/></URLTemplate>
</ChildObjects></HTTPService></MetaDataObject>"#
        );
        let registration = owner(&[("HTTPService", "Api")]);
        let duplicate = subject("HTTPService.Api", Some(&descriptor), &registration, &[]);

        let result =
            assert_internal_status("duplicate-child", &duplicate, MetaValidationStatus::Failed);

        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Duplicate URLTemplate")));
    }

    #[test]
    fn event_subscription_defined_type_source_is_not_reported_as_empty() {
        let document = Document::parse(
            r#"<Properties xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config"><Handler>CommonModule.Target.Run</Handler><Source><v8:TypeSet>cfg:DefinedType.Identifier</v8:TypeSet></Source></Properties>"#,
        )
        .unwrap();
        let mut report = MetaValidationReporter::new(30);

        meta_validate_check_cross_properties(
            &mut report,
            "EventSubscription",
            Some(document.root_element()),
            None,
            None,
            None,
            "Events",
        );

        assert!(report.errors.is_empty());
        assert!(report
            .warnings
            .iter()
            .all(|warning| !warning.contains("no Source types specified")));
    }

    #[test]
    fn hierarchy_type_is_not_checked_on_a_non_hierarchical_catalog() {
        let registration = owner(&[("Catalog", "Stores")]);
        let catalog = |hierarchical: &str, hierarchy_type: &str| {
            format!(
                r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<Catalog uuid="66666666-6666-4666-8666-666666666666">
<Properties><Name>Stores</Name><Hierarchical>{hierarchical}</Hierarchical><HierarchyType>{hierarchy_type}</HierarchyType></Properties><ChildObjects/>
</Catalog></MetaDataObject>"#
            )
        };
        let hierarchy_findings = |hierarchical: &str, hierarchy_type: &str| {
            let descriptor = catalog(hierarchical, hierarchy_type);
            let proof = subject("Catalog.Stores", Some(&descriptor), &registration, &[]);
            let result =
                MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                    unica_forms_designer::root::require_form_root,
                ))
                .validate(&proof, &context());
            let findings = result
                .diagnostics
                .into_iter()
                .filter(|item| item.message.contains("HierarchyType"))
                .collect::<Vec<_>>();
            (result.status, findings)
        };

        // The pair every 8.3.27 dump and the catalog template write for a
        // non-hierarchical catalog, and a valid hierarchical catalog, are clean.
        for (hierarchical, hierarchy_type) in [
            ("false", "HierarchyFoldersAndItems"),
            ("true", "HierarchyFoldersAndItems"),
            ("true", "HierarchyOfItems"),
        ] {
            let (_, findings) = hierarchy_findings(hierarchical, hierarchy_type);
            assert!(
                findings.is_empty(),
                "Hierarchical={hierarchical}, HierarchyType={hierarchy_type}: {findings:?}"
            );
        }

        // HierarchyType is inactive on a non-hierarchical catalog, so no value
        // there is a finding.
        let (_, findings) = hierarchy_findings("false", "HierarchyOfItems");
        assert!(findings.is_empty(), "{findings:?}");

        // An invalid type on a hierarchical catalog is still a validation error.
        let (status, findings) = hierarchy_findings("true", "HierarchyOfFolders");
        assert_eq!(status, MetaValidationStatus::Failed);
        assert!(
            findings.iter().any(|item| {
                item.severity == MetaDiagnosticSeverity::Error
                    && item.code == MetaDiagnosticCode::ValidationFailed
                    && item.message.contains("'HierarchyOfFolders'")
            }),
            "{findings:?}"
        );
    }

    #[test]
    fn internal_invalid_reference_failure_matches_legacy_classification() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<DocumentJournal uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>Journal</Name></Properties><ChildObjects>
<Column uuid="44444444-4444-4444-8444-444444444444"><Properties><Name>Broken</Name><References/></Properties></Column>
</ChildObjects></DocumentJournal></MetaDataObject>"#
        );
        let registration = owner(&[("DocumentJournal", "Journal")]);
        let invalid = subject(
            "DocumentJournal.Journal",
            Some(&descriptor),
            &registration,
            &[],
        );

        let result =
            assert_internal_status("invalid-reference", &invalid, MetaValidationStatus::Failed);

        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("empty References")));
    }

    #[test]
    fn internal_method_validation_associates_export_with_the_referenced_common_module() {
        let descriptor = scheduled_job("Nightly", "CommonModule.Target.Run");
        let registration = owner(&[
            ("ScheduledJob", "Nightly"),
            ("CommonModule", "Target"),
            ("CommonModule", "Wrong"),
        ]);
        let target_module = common_module("Target");
        let wrong_module = common_module("Wrong");
        let mut resources = subject(
            "ScheduledJob.Nightly",
            Some(&descriptor),
            &registration,
            &[
                ("CommonModule.Target", &target_module),
                ("CommonModule.Wrong", &wrong_module),
            ],
        )
        .resources;
        resources.push(image(
            unica_metadata::validation_subject::MetadataResourceRole::Module {
                owner: address("CommonModule.Wrong"),
            },
            b"Procedure Run() Export\nEndProcedure".to_vec(),
        ));
        resources.push(image(
            unica_metadata::validation_subject::MetadataResourceRole::Module {
                owner: address("CommonModule.Target"),
            },
            b"Procedure Different() Export\nEndProcedure".to_vec(),
        ));
        let inaccessible = unica_metadata::validation_subject::MetadataValidationSubject {
            target: address("ScheduledJob.Nightly"),
            resources,
            child_footprints: Vec::new(),
            registrar_evidence: Default::default(),
            subsystem_evidence: Default::default(),
        };

        let result =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(&inaccessible, &context());
        assert_eq!(result.status, MetaValidationStatus::Failed);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == MetaDiagnosticSeverity::Error
                && diagnostic.message.contains("not found as exported")
        }));

        let mut accessible = inaccessible.clone();
        accessible.resources.pop();
        accessible.resources.push(image(
            unica_metadata::validation_subject::MetadataResourceRole::Module {
                owner: address("CommonModule.Target"),
            },
            "\u{feff}Procedure Run() Export\nEndProcedure"
                .as_bytes()
                .to_vec(),
        ));
        let result =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(&accessible, &context());
        assert_eq!(result.status, MetaValidationStatus::Passed);
        assert!(!result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("not found as exported")));
    }

    #[test]
    fn scheduled_job_method_validation_uses_exact_exported_procedure_without_required_arguments() {
        for (case, method, source, expected) in [
            ("zero", "Run", "Procedure Run() Export\nEndProcedure\n", MetaValidationStatus::Passed),
            ("optional-en", "Run", "Procedure Run(Profile = Undefined, Manual = False) Export\nEndProcedure\n", MetaValidationStatus::Passed),
            ("optional-ru-multiline", "Запустить", "Процедура Запустить(\n    Профиль = Неопределено,\n    РучнойЗапуск = Ложь\n) Экспорт\nКонецПроцедуры\n", MetaValidationStatus::Passed),
            ("optional-en-multiline", "Run", "Procedure Run(\n    Profile = Undefined,\n    Manual = False\n) Export\nEndProcedure\n", MetaValidationStatus::Passed),
            ("case-insensitive", "run", "PROCEDURE RUN(Value = 0) EXPORT\nENDPROCEDURE\n", MetaValidationStatus::Passed),
            ("unicode-case", "запустить", "Процедура ЗАПУСТИТЬ(Ручной = Ложь) Экспорт\nКонецПроцедуры\n", MetaValidationStatus::Passed),
            ("required", "Run", "Procedure Run(Profile) Export\nEndProcedure\n", MetaValidationStatus::Failed),
            ("mixed", "Run", "Procedure Run(Profile, Manual = False) Export\nEndProcedure\n", MetaValidationStatus::Failed),
            ("private", "Run", "Procedure Run(Profile = Undefined)\nEndProcedure\n", MetaValidationStatus::Failed),
            ("function", "Run", "Function Run(Profile = Undefined) Export\nReturn True;\nEndFunction\n", MetaValidationStatus::Failed),
            ("prefix-lookalike", "Run", "Procedure RunAnother() Export\nEndProcedure\n", MetaValidationStatus::Failed),
            ("comment-lookalike", "Run", "// Procedure Run() Export\nProcedure Other() Export\nEndProcedure\n", MetaValidationStatus::Failed),
            ("malformed", "Run", "Procedure Run(Profile = ) Export\nEndProcedure\n", MetaValidationStatus::Failed),
        ] {
            let descriptor = scheduled_job("Nightly", &format!("CommonModule.Target.{method}"));
            let registration = owner(&[("ScheduledJob", "Nightly"), ("CommonModule", "Target")]);
            let target_module = common_module("Target");
            let mut candidate = subject(
                "ScheduledJob.Nightly",
                Some(&descriptor),
                &registration,
                &[("CommonModule.Target", &target_module)],
            );
            candidate.resources.push(image(
                unica_metadata::validation_subject::MetadataResourceRole::Module { owner: address("CommonModule.Target") },
                source.as_bytes().to_vec(),
            ));
            let before: Vec<_> = candidate.resources.iter().map(|resource| resource.bytes.clone()).collect();
            let result = MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(unica_forms_designer::root::require_form_root)).validate(&candidate, &context());
            assert_eq!(result.status, expected, "{case}: {source}\n{:?}", result.diagnostics);
            if expected == MetaValidationStatus::Failed {
                assert!(result.diagnostics.iter().any(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error), "{case}: refusal needs an error diagnostic");
            }
            assert_eq!(candidate.resources.iter().map(|resource| resource.bytes.clone()).collect::<Vec<_>>(), before, "{case}: validation must not rewrite source evidence");
        }
    }

    #[test]
    fn internal_missing_method_dependency_matches_real_legacy_failure() {
        let descriptor = scheduled_job("Nightly", "CommonModule.Missing.Run");
        let registration = owner(&[("ScheduledJob", "Nightly")]);
        let missing = subject(
            "ScheduledJob.Nightly",
            Some(&descriptor),
            &registration,
            &[],
        );

        let result = assert_internal_status(
            "missing-method-dependency",
            &missing,
            MetaValidationStatus::Failed,
        );
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("CommonModule `Missing`")));
    }

    #[test]
    fn internal_method_validation_reports_malformed_module_reference() {
        let descriptor = scheduled_job("Nightly", "CommonModule..Run");
        let registration = owner(&[("ScheduledJob", "Nightly")]);
        let malformed = subject(
            "ScheduledJob.Nightly",
            Some(&descriptor),
            &registration,
            &[],
        );

        let result =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(&malformed, &context());

        assert_eq!(result.status, MetaValidationStatus::Failed);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("expected format 'CommonModule.ModuleName.ProcedureName'")
        }));
    }

    #[test]
    fn internal_corrupt_method_dependency_matches_real_legacy_provider_failure() {
        let descriptor = scheduled_job("Nightly", "CommonModule.Target.Run");
        let registration = owner(&[("ScheduledJob", "Nightly"), ("CommonModule", "Target")]);
        let mut invalid = subject(
            "ScheduledJob.Nightly",
            Some(&descriptor),
            &registration,
            &[("CommonModule.Target", "not XML")],
        );
        invalid.resources.push(image(
            unica_metadata::validation_subject::MetadataResourceRole::Module {
                owner: address("CommonModule.Target"),
            },
            b"Procedure Run() Export\nEndProcedure".to_vec(),
        ));

        let result = assert_internal_status(
            "corrupt-method-dependency",
            &invalid,
            MetaValidationStatus::Failed,
        );

        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == MetaDiagnosticCode::ProviderUnavailable));
    }

    /// Платформа 8.3.27 выгружает ссылку расширения на тип расширяемой
    /// конфигурации как `v8:TypeId`: один и после `v8:Type` в составном
    /// типе, в том числе у реквизита табличной части (#1361). Фикстура —
    /// выгрузка `ibcmd config export --extension`.
    #[test]
    fn extension_type_id_reference_is_a_valid_type_block() {
        let dump = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/acceptance/workspace-format/src-ext/Catalogs/ПР_НастройкиВалют.xml"
        ));
        let options = MetaValidationOptions { max_errors: 30 };
        let inputs = MetaValidationReferenceInputs {
            config_dir: None,
            language_codes: Vec::new(),
            proof_subject: None,
        };
        let validate = |xml: &str| meta_validate_source(xml.as_bytes(), &options, &inputs).unwrap();
        let currency = "<v8:TypeId>41dbb66b-ff77-4b8f-aaf8-0ca2012d3a6f</v8:TypeId>";
        let rate_mode = "<v8:TypeId>d7bc4697-f470-58f7-aca4-4e6bf45f9e69</v8:TypeId>";
        assert_eq!(
            dump.matches(currency).count(),
            3,
            "composite, single and tabular section"
        );
        assert_eq!(dump.matches(rate_mode).count(), 2, "composite and single");

        let run = validate(dump);
        assert!(run.ok, "{:?}", run.errors);
        assert!(
            run.errors.is_empty() && run.warnings.is_empty(),
            "{:?} {:?}",
            run.errors,
            run.warnings
        );

        let malformed = validate(&dump.replace(rate_mode, "<v8:TypeId>not-a-uuid</v8:TypeId>"));
        assert!(!malformed.ok);
        assert!(
            malformed
                .errors
                .iter()
                .any(|error| error.contains("has invalid v8:TypeId 'not-a-uuid'")),
            "{:?}",
            malformed.errors
        );

        let duplicate = validate(&dump.replacen(
            currency,
            &format!(
                "{currency}{}",
                currency.to_uppercase().replace("V8:TYPEID", "v8:TypeId")
            ),
            1,
        ));
        assert!(
            duplicate
                .errors
                .iter()
                .any(|error| error.contains("has duplicate v8:TypeId")),
            "{:?}",
            duplicate.errors
        );

        let empty = validate(&dump.replace(currency, "").replace(rate_mode, ""));
        assert!(!empty.ok);
        for name in ["Attribute 'ОсновнаяВалюта'", "Attribute 'КодЯзыка'"] {
            assert!(
                empty
                    .errors
                    .iter()
                    .any(|error| error.contains(name) && error.contains("Type block has no")),
                "{name}: {:?}",
                empty.errors
            );
        }
    }

    /// Source подписки расширения из штатной выгрузки 8.3.27 содержит
    /// `v8:TypeId` источника из основной конфигурации после собственного
    /// `v8:Type` (#1361) — это допустимо. Source из одних `v8:TypeId`
    /// платформа не загружает («Имя события должно быть задано»).
    #[test]
    fn extension_event_subscription_type_id_source_needs_an_own_source() {
        let dump = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/acceptance/workspace-format/src-ext/EventSubscriptions/ПР_ПередЗаписьюНастроек.xml"
        ));
        let options = MetaValidationOptions { max_errors: 30 };
        let inputs = MetaValidationReferenceInputs {
            config_dir: None,
            language_codes: Vec::new(),
            proof_subject: None,
        };
        let own = "<v8:Type>cfg:CatalogObject.ПР_НастройкиВалют</v8:Type>";
        assert!(dump.contains(own) && dump.contains("<v8:TypeId>"));

        let run = meta_validate_source(dump.as_bytes(), &options, &inputs).unwrap();
        assert!(run.ok, "{:?}", run.errors);
        assert!(run.warnings.is_empty(), "{:?}", run.warnings);

        let only_type_id = dump.replace(own, "");
        let run = meta_validate_source(only_type_id.as_bytes(), &options, &inputs).unwrap();
        assert!(!run.ok);
        assert!(
            run.errors
                .iter()
                .any(|error| error.contains("Source has only v8:TypeId")),
            "{:?}",
            run.errors
        );
    }

    #[test]
    fn internal_reverse_registrar_inconsistency_is_not_lost() {
        let descriptor = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/InformationRegisters/SubordinateRegister.xml"
        ));
        let registration = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Configuration.xml"
        ));
        let language = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Languages/Русский.xml"
        ));
        let registrar = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Documents/Регистратор.xml"
        ))
        .replace(
            "InformationRegister.SubordinateRegister",
            "InformationRegister.Other",
        );
        let invalid = subject(
            "InformationRegister.SubordinateRegister",
            Some(descriptor),
            registration,
            &[
                ("Language.Русский", language),
                ("Document.Регистратор", &registrar),
            ],
        );

        let result =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(&invalid, &context());

        assert_eq!(result.status, MetaValidationStatus::Passed);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == MetaDiagnosticSeverity::Warning
                && diagnostic.message.contains("no registrar document found")
        }));

        let complete_read =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate_complete_read(&invalid, &context());
        assert_eq!(complete_read.status, MetaValidationStatus::Failed);
        assert!(complete_read.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == MetaDiagnosticSeverity::Error
                && diagnostic.code == MetaDiagnosticCode::ValidationFailed
                && diagnostic.metadata_path.as_ref() == Some(&invalid.target)
                && diagnostic.field.as_deref() == Some("resources")
        }));
    }

    #[test]
    fn registrar_evidence_availability_is_strict_read_only() {
        let descriptor = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/InformationRegisters/SubordinateRegister.xml"
        ));
        let registration = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Configuration.xml"
        ));
        let language = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Languages/Русский.xml"
        ));
        let mut unavailable = subject(
            "InformationRegister.SubordinateRegister",
            Some(descriptor),
            registration,
            &[("Language.Русский", language)],
        );
        unavailable.registrar_evidence =
            unica_metadata::validation_subject::MetadataEvidenceAvailability::Unavailable(vec![
                MetaDiagnostic::error(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "registrar evidence is unavailable",
                )
                .with_metadata_path(unavailable.target.clone())
                .with_field("registrarEvidence"),
            ]);

        let legacy =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(&unavailable, &context());
        assert_eq!(legacy.status, MetaValidationStatus::Passed);
        assert!(legacy.diagnostics.iter().all(|diagnostic| {
            diagnostic.severity != MetaDiagnosticSeverity::Error
                || diagnostic.code != MetaDiagnosticCode::ProviderUnavailable
        }));

        let strict =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate_complete_read(&unavailable, &context());
        assert_eq!(strict.status, MetaValidationStatus::Failed);
        assert!(strict.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == MetaDiagnosticSeverity::Error
                && diagnostic.code == MetaDiagnosticCode::ProviderUnavailable
                && diagnostic.field.as_deref() == Some("registrarEvidence")
        }));
        assert!(strict.diagnostics.iter().all(|diagnostic| {
            diagnostic.severity != MetaDiagnosticSeverity::Error
                || diagnostic.code != MetaDiagnosticCode::ValidationFailed
        }));
    }

    #[test]
    fn complete_read_proof_accepts_consistent_forward_and_reverse_dependencies() {
        let document = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Documents/Регистратор.xml"
        ));
        let register = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/InformationRegisters/SubordinateRegister.xml"
        ));
        let registration = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Configuration.xml"
        ));
        let language = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Languages/Русский.xml"
        ));
        let document_subject = subject(
            "Document.Регистратор",
            Some(document),
            registration,
            &[
                ("Language.Русский", language),
                ("InformationRegister.SubordinateRegister", register),
            ],
        );
        let register_subject = subject(
            "InformationRegister.SubordinateRegister",
            Some(register),
            registration,
            &[
                ("Language.Русский", language),
                ("Document.Регистратор", document),
            ],
        );

        assert!(complete_read_proof_diagnostics(&document_subject).is_empty());
        assert!(complete_read_proof_diagnostics(&register_subject).is_empty());
    }

    #[test]
    fn internal_declared_resource_encodings_fail_closed() {
        let descriptor = common_module("Service");
        let registration = owner(&[("CommonModule", "Service")]);
        let corrupt_roles = [
            unica_metadata::validation_subject::MetadataResourceRole::Dependency {
                target: address("Subsystem.Area"),
            },
            unica_metadata::validation_subject::MetadataResourceRole::Module {
                owner: address("CommonModule.Other"),
            },
            unica_metadata::validation_subject::MetadataResourceRole::Form {
                owner: address("CommonModule.Service"),
                name: "Main".to_string(),
            },
            unica_metadata::validation_subject::MetadataResourceRole::Template {
                owner: address("CommonModule.Service"),
                name: "Layout".to_string(),
            },
            unica_metadata::validation_subject::MetadataResourceRole::Command {
                owner: address("CommonModule.Service"),
                name: "Execute".to_string(),
            },
            unica_metadata::validation_subject::MetadataResourceRole::AuxiliaryXml {
                owner: address("ExchangePlan.Sync"),
                kind: unica_metadata::validation_subject::MetadataAuxiliaryXmlKind::ExchangePlanContent,
            },
        ];
        for role in corrupt_roles {
            let mut invalid = subject(
                "CommonModule.Service",
                Some(&descriptor),
                &registration,
                &[],
            );
            let bytes = if matches!(
                role,
                unica_metadata::validation_subject::MetadataResourceRole::Module { .. }
            ) {
                vec![0xff, 0xfe]
            } else {
                b"not XML".to_vec()
            };
            invalid.resources.push(image(role, bytes));

            let result =
                MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                    unica_forms_designer::root::require_form_root,
                ))
                .validate(&invalid, &context());

            assert_eq!(result.status, MetaValidationStatus::Failed);
            assert!(result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == MetaDiagnosticCode::ProviderUnavailable));
        }
    }

    #[test]
    fn internal_remove_validates_surviving_images_without_deleted_descriptor() {
        let clean_registration = owner(&[("CommonModule", "Other")]);
        let clean_dependency = dependency_with_reference(Some("CommonModule.Other"));
        let clean = subject(
            "CommonModule.Service",
            None,
            &clean_registration,
            &[("Subsystem.Area", &clean_dependency)],
        );
        assert_eq!(
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root
            ))
            .validate(&clean, &context())
            .status,
            MetaValidationStatus::Passed
        );

        let stale_registration = owner(&[("CommonModule", "Service")]);
        let registration_failure = subject(
            "CommonModule.Service",
            None,
            &stale_registration,
            &[("Subsystem.Area", &clean_dependency)],
        );
        let result =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(&registration_failure, &context());
        assert_eq!(result.status, MetaValidationStatus::Failed);
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("surviving registration")));

        let stale_dependency = dependency_with_reference(Some("CommonModule.Service"));
        let dependency_failure = subject(
            "CommonModule.Service",
            None,
            &clean_registration,
            &[("Subsystem.Area", &stale_dependency)],
        );
        let result =
            MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ))
            .validate(&dependency_failure, &context());
        assert_eq!(result.status, MetaValidationStatus::Failed);
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("surviving reference")));
    }

    #[test]
    fn internal_remove_rejects_every_corrupt_dependency_encoding() {
        let clean_registration = owner(&[("CommonModule", "Other")]);
        for bytes in [vec![0xff, 0xfe], b"not XML".to_vec(), b"<broken".to_vec()] {
            let invalid = unica_metadata::validation_subject::MetadataValidationSubject {
                target: address("CommonModule.Service"),
                resources: vec![
                    image(
                        unica_metadata::validation_subject::MetadataResourceRole::Registration,
                        clean_registration.as_bytes().to_vec(),
                    ),
                    image(
                        unica_metadata::validation_subject::MetadataResourceRole::Dependency {
                            target: address("Subsystem.Area"),
                        },
                        bytes,
                    ),
                ],
                child_footprints: Vec::new(),
                registrar_evidence: Default::default(),
                subsystem_evidence: Default::default(),
            };

            let result =
                MetadataValidator::new(unica_metadata_designer::validation::FormRootValidator(
                    unica_forms_designer::root::require_form_root,
                ))
                .validate(&invalid, &context());

            assert_eq!(result.status, MetaValidationStatus::Failed);
            assert!(result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == MetaDiagnosticCode::ProviderUnavailable));
        }
    }

    fn check_services_errors(http_methods: &[&str]) -> Vec<String> {
        let methods = http_methods
            .iter()
            .enumerate()
            .map(|(index, literal)| {
                format!(
                    "<Method><Properties><Name>Метод{index}</Name><HTTPMethod>{literal}</HTTPMethod><Handler>Обработчик{index}</Handler></Properties></Method>"
                )
            })
            .collect::<String>();
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><HTTPService><Properties><Name>ПВД_ПечатныеФормы</Name></Properties><ChildObjects><URLTemplate><Properties><Name>Корень</Name><Template>/*</Template></Properties><ChildObjects>{methods}</ChildObjects></URLTemplate></ChildObjects></HTTPService></MetaDataObject>"#
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let type_node = document.root_element().first_element_child().unwrap();
        let mut report = MetaValidationReporter::new(30);
        meta_validate_check_services(
            &mut report,
            "HTTPService",
            unica_xml_format::read::child_element(type_node, "ChildObjects"),
        );
        report.errors
    }

    #[test]
    fn check_services_accepts_the_full_8_3_27_http_method_enum() {
        let errors = check_services_errors(&["Any", "PROPFIND", "MKCOL", "UNLOCK", "GET"]);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn check_services_rejects_the_uppercase_any_literal() {
        let errors = check_services_errors(&["ANY"]);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("invalid HTTPMethod 'ANY'")),
            "{errors:?}"
        );
    }

    #[test]
    fn service_enum_authority_is_the_8_3_27_property_table() {
        for (enum_name, actual) in [
            (
                "HTTPMethod",
                unica_metadata_designer::validation::meta_validate_valid_http_methods(),
            ),
            (
                "TransferDirection",
                meta_validate_valid_transfer_directions(),
            ),
        ] {
            let expected = unica_metadata_designer::validation::meta_validate_property_values()
                .iter()
                .find(|(name, _)| *name == enum_name)
                .map(|(_, allowed)| *allowed)
                .unwrap();
            assert_eq!(actual, expected, "{enum_name}");
        }
    }
}

#[cfg(test)]
mod template_removal_owner_range_tests {
    use super::*;

    #[test]
    fn owner_ranges_use_exact_utf8_basis_and_ignore_foreign_reference_shadows() {
        let original = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/acceptance/workspace-dcs/src/cf/Reports/F05Report.xml"
        ));
        let descriptor = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/acceptance/workspace-dcs/src/cf/Reports/F05Report/Templates/F05Schema.xml"));
        let start = "<Properties><Name>F05Report</Name>";
        let reference = "Report.F05Report.Template.F05Schema";
        let encoded = "Report.F05Report.Template.F05Sch&#101;ma";
        assert_eq!(original.matches(start).count(), 1);
        let properties = format!(
            "{start}<!--До: {reference}--><foreign:MainDataCompositionSchema xmlns:foreign=\"urn:foreign\">{reference}</foreign:MainDataCompositionSchema><actual:MainDataCompositionSchema xmlns:actual=\"http://v8.1c.ru/8.3/MDClasses\"><!--сохранить-->{encoded}</actual:MainDataCompositionSchema>"
        );
        let source = format!(
            "\u{feff}\u{feff}{}",
            original.replacen(start, &properties, 1)
        );
        let owner =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Report.F05Report").unwrap();
        let identity =
            validate_template_removal_identity(&source, descriptor, &owner, "F05Schema", false)
                .unwrap();
        assert_eq!(
            &source[identity.registration],
            "<Template>F05Schema</Template>"
        );
        assert_eq!(
            identity.template_type.as_deref(),
            Some("DataCompositionSchema")
        );
        assert_eq!(identity.reference_edits.len(), 1);
        let (range, replacement) = &identity.reference_edits[0];
        assert_eq!(&source[range.clone()], encoded);
        assert!(replacement.is_empty());
    }
}
