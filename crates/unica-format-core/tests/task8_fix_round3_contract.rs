use serde_json::json;
use unica_format_core::commands::*;

fn text<T>(value: &str, constructor: impl FnOnce(String) -> Result<T, SemanticValueError>) -> T {
    constructor(value.to_string()).unwrap()
}

fn property_value(name: MetadataKindPropertyName) -> MetadataPropertyValue {
    use MetadataKindPropertyName as Name;
    use MetadataPropertyValue as Value;

    match name {
        Name::Hierarchical
        | Name::LimitLevelCount
        | Name::FoldersOnTop
        | Name::CheckUnique
        | Name::Autonumbering
        | Name::QuickChoice
        | Name::SequenceFilling
        | Name::PostInPrivilegedMode
        | Name::UnpostInPrivilegedMode
        | Name::MainFilterOnPeriod
        | Name::EnableTotalsSplitting
        | Name::Correspondence
        | Name::ActionPeriod
        | Name::BasePeriod
        | Name::AutoOrderByCode
        | Name::ActionPeriodUse
        | Name::DistributedInfoBase
        | Name::IncludeConfigurationExtensions
        | Name::Nonnegative
        | Name::CreateTaskInPrivilegedMode
        | Name::Use
        | Name::Predefined => Value::Boolean(true),
        Name::LevelCount
        | Name::CodeLength
        | Name::DescriptionLength
        | Name::NumberLength
        | Name::PeriodAdjustmentLength
        | Name::MaxExtDimensionCount
        | Name::OrderLength
        | Name::RestartCountOnFailure
        | Name::RestartIntervalOnFailure
        | Name::SessionMaxAge
        | Name::Length
        | Name::Precision => Value::Integer(1),
        Name::CodeMask | Name::Description | Name::MainAddressingAttribute => {
            Value::Text(text("value", MetadataPropertyText::new))
        }
        Name::ValueType | Name::Addressing => {
            Value::Type(MetadataTypeExpression::String { length: Some(10) })
        }
        Name::ValueTypes => Value::Types(vec![MetadataTypeExpression::Boolean]),
        Name::Context => Value::ModuleContext(MetadataModuleContext::Server),
        Name::ReturnValuesReuse => {
            Value::ReturnValuesReuse(MetadataReturnValuesReuse::DuringRequest)
        }
        Name::HierarchyType => Value::HierarchyType(MetadataHierarchyType::GroupsAndItems),
        Name::Periodicity => Value::Periodicity(MetadataPeriodicity::Day),
        Name::RegisterType => Value::RegisterKind(MetadataRegisterKind::Balance),
        Name::ChartOfAccounts
        | Name::ChartOfCalculationTypes
        | Name::ExtDimensionTypes
        | Name::Task => Value::Object(text("Catalog.Items", MetadataObjectReference::new)),
        Name::AccountingFlags | Name::ExtDimensionAccountingFlags => {
            Value::Texts(vec![text("Flag", MetadataPropertyText::new)])
        }
        Name::DependenceOnCalculationTypes => {
            Value::CalculationDependence(MetadataCalculationDependence::OnActionPeriod)
        }
        Name::BaseCalculationTypes | Name::RegisteredDocuments | Name::Source => {
            Value::Objects(vec![text("Document.Order", MetadataObjectReference::new)])
        }
        Name::MethodName | Name::Handler => {
            Value::Method(text("Module.Method", MetadataMethodReference::new))
        }
        Name::Key => Value::JobKey(text("job", MetadataJobKey::new)),
        Name::Event => Value::Event(text("BeforeWrite", MetadataEventName::new)),
        Name::RootUrl => Value::UrlRoot(text("/service", MetadataUrlRoot::new)),
        Name::ReuseSessions => Value::SessionReuse(MetadataSessionReuse::Automatic),
        Name::UrlTemplates => Value::UrlTemplates(Vec::new()),
        Name::Namespace => Value::ServiceNamespace(text("urn:test", MetadataServiceNamespace::new)),
        Name::Operations => Value::ServiceOperations(Vec::new()),
    }
}

fn expected_properties(kind: MetadataKind) -> &'static [MetadataKindPropertyName] {
    use MetadataKind as Kind;
    use MetadataKindPropertyName as P;

    match kind {
        Kind::CommonModule => &[P::Context, P::ReturnValuesReuse],
        Kind::SessionParameter | Kind::CommonAttribute | Kind::FilterCriterion | Kind::Constant => {
            &[P::Length, P::Precision, P::Nonnegative, P::ValueType]
        }
        Kind::FunctionalOption | Kind::FunctionalOptionsParameter => &[
            P::Length,
            P::Precision,
            P::Nonnegative,
            P::ValueType,
            P::Description,
        ],
        Kind::Role
        | Kind::XdtoPackage
        | Kind::WsReference
        | Kind::CommonPicture
        | Kind::CommonTemplate
        | Kind::SettingsStorage
        | Kind::Language
        | Kind::Sequence
        | Kind::Report
        | Kind::DataProcessor => &[],
        Kind::ExchangePlan => &[
            P::CodeLength,
            P::DescriptionLength,
            P::CheckUnique,
            P::Autonumbering,
            P::QuickChoice,
            P::DistributedInfoBase,
            P::IncludeConfigurationExtensions,
        ],
        Kind::WebService => &[
            P::SessionMaxAge,
            P::ReuseSessions,
            P::Namespace,
            P::Operations,
        ],
        Kind::HttpService => &[
            P::SessionMaxAge,
            P::ReuseSessions,
            P::RootUrl,
            P::UrlTemplates,
        ],
        Kind::StyleItem => &[P::ValueType, P::Description],
        Kind::EventSubscription => &[P::Source, P::Event, P::Handler],
        Kind::ScheduledJob => &[
            P::RestartCountOnFailure,
            P::RestartIntervalOnFailure,
            P::MethodName,
            P::Description,
            P::Key,
            P::Use,
            P::Predefined,
        ],
        Kind::DefinedType => &[P::ValueTypes],
        Kind::CommandGroup | Kind::CommonCommand => &[P::Description],
        Kind::DocumentNumerator => &[P::NumberLength, P::CheckUnique, P::Autonumbering],
        Kind::Catalog => &[
            P::Hierarchical,
            P::LimitLevelCount,
            P::LevelCount,
            P::FoldersOnTop,
            P::CodeLength,
            P::DescriptionLength,
            P::CheckUnique,
            P::Autonumbering,
            P::QuickChoice,
            P::HierarchyType,
        ],
        Kind::Document => &[
            P::NumberLength,
            P::CheckUnique,
            P::Autonumbering,
            P::SequenceFilling,
            P::PostInPrivilegedMode,
            P::UnpostInPrivilegedMode,
        ],
        Kind::Enum => &[P::QuickChoice],
        Kind::ChartOfCharacteristicTypes => &[
            P::CodeLength,
            P::DescriptionLength,
            P::CheckUnique,
            P::Autonumbering,
            P::QuickChoice,
            P::ValueTypes,
        ],
        Kind::ChartOfAccounts => &[
            P::Hierarchical,
            P::LimitLevelCount,
            P::LevelCount,
            P::FoldersOnTop,
            P::CodeLength,
            P::DescriptionLength,
            P::CheckUnique,
            P::Autonumbering,
            P::QuickChoice,
            P::MaxExtDimensionCount,
            P::CodeMask,
            P::AutoOrderByCode,
            P::OrderLength,
            P::ExtDimensionTypes,
            P::AccountingFlags,
            P::ExtDimensionAccountingFlags,
        ],
        Kind::ChartOfCalculationTypes => &[
            P::CodeLength,
            P::DescriptionLength,
            P::CheckUnique,
            P::Autonumbering,
            P::QuickChoice,
            P::ActionPeriodUse,
            P::DependenceOnCalculationTypes,
            P::BaseCalculationTypes,
        ],
        Kind::InformationRegister => &[P::MainFilterOnPeriod, P::Periodicity],
        Kind::AccumulationRegister => &[P::EnableTotalsSplitting, P::RegisterType],
        Kind::AccountingRegister => &[P::Correspondence, P::ChartOfAccounts],
        Kind::CalculationRegister => &[
            P::Periodicity,
            P::PeriodAdjustmentLength,
            P::ActionPeriod,
            P::BasePeriod,
            P::ChartOfCalculationTypes,
        ],
        Kind::BusinessProcess => &[
            P::NumberLength,
            P::CheckUnique,
            P::Autonumbering,
            P::CreateTaskInPrivilegedMode,
            P::Task,
        ],
        Kind::Task => &[
            P::NumberLength,
            P::CheckUnique,
            P::Autonumbering,
            P::Addressing,
            P::MainAddressingAttribute,
        ],
        Kind::DocumentJournal => &[P::RegisteredDocuments],
    }
}

#[test]
fn metadata_kind_property_applicability_is_exhaustive_and_serde_enforced() {
    let mut checked = 0;
    for kind in MetadataKind::ALL {
        for name in MetadataKindPropertyName::ALL {
            let expected = expected_properties(kind).contains(&name);
            assert_eq!(
                metadata_kind_allows_property(kind, name),
                expected,
                "{kind:?} + {name:?}"
            );
            let property = MetadataKindProperty::new(name, property_value(name)).unwrap();
            let definition = MetadataKindDefinition::new(kind, vec![property]);
            assert_eq!(definition.is_ok(), expected, "{kind:?} + {name:?}");

            let wire = json!({
                "kind": kind,
                "properties": [{"name": name, "value": property_value(name)}]
            });
            assert_eq!(
                serde_json::from_value::<MetadataKindDefinition>(wire).is_ok(),
                expected,
                "serde {kind:?} + {name:?}"
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked,
        MetadataKind::ALL.len() * MetadataKindPropertyName::ALL.len()
    );

    assert!(MetadataKindDefinition::new(
        MetadataKind::Catalog,
        vec![MetadataKindProperty::new(
            MetadataKindPropertyName::Periodicity,
            MetadataPropertyValue::Periodicity(MetadataPeriodicity::Month),
        )
        .unwrap()],
    )
    .is_err());
}

#[test]
fn capability_requirement_is_ordered_closed_and_rejects_invalid_wire_shapes() {
    let older = VersionNumber::new(vec![8, 3, 24]).unwrap();
    let newer = VersionNumber::new(vec![8, 3, 27]).unwrap();
    assert!(older < newer);
    let requirement = CapabilityRequirement::Explicit(newer);
    let wire = serde_json::to_value(&requirement).unwrap();
    assert_eq!(
        serde_json::from_value::<CapabilityRequirement>(wire).unwrap(),
        requirement
    );
    for invalid in [
        json!({"selection": "explicit", "requirement": {"components": [8]}}),
        json!({"selection": "explicit", "requirement": {"components": [0, 0]}}),
        json!({"selection": "explicit", "requirement": {"components": [8, 3, 27], "native": "Version8_3_27"}}),
        json!({"selection": "future"}),
    ] {
        assert!(serde_json::from_value::<CapabilityRequirement>(invalid).is_err());
    }
}

#[test]
fn extension_initialization_round_trips_every_public_semantic_field() {
    let command = ExtensionInitialize::new(text("Audit", ExtensionName::new))
        .with_synonym(Some(text("Audit extension", SynonymText::new)))
        .with_purpose(Some(ExtensionPurpose::AddOn))
        .with_prefix(Some(text("AUD_", NamePrefix::new)))
        .with_vendor(Some(text("Vendor", VendorName::new)))
        .with_version(Some(text("4.2.1", ArtifactVersion::new)))
        .omit_default_role(true)
        .with_compatibility(CapabilityRequirement::Explicit(
            VersionNumber::new(vec![8, 3, 25]).unwrap(),
        ));
    let wire = serde_json::to_value(&command).unwrap();
    let decoded: ExtensionInitialize = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded, command);
    assert_eq!(decoded.vendor().unwrap().as_str(), "Vendor");
    assert_eq!(decoded.version().unwrap().as_str(), "4.2.1");
    assert!(decoded.omits_default_role());
}
