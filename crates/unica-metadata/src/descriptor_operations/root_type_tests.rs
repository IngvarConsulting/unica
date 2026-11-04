use super::*;

#[test]
fn root_type_profiles_advertise_type_without_granting_neighboring_properties() {
    for kind in [
        "CommonAttribute",
        "SessionParameter",
        "FilterCriterion",
        "DefinedType",
    ] {
        let at = QualifiedAddress::parse(&format!("main:{kind}.Existing")).unwrap();
        let site = PropertiesSite::from_address(&at)
            .unwrap_or_else(|| panic!("{kind} must have an existing-object Type profile"));
        let contract = PropertiesOperation::Set.contract(site);
        let properties = contract
            .schema
            .pointer("/properties/values/properties")
            .unwrap();
        assert!(properties.get("Type").is_some(), "{kind}: {properties}");
        if kind == "DefinedType" {
            assert!(properties.get("Synonym").is_some());
            assert!(properties.get("Comment").is_some());
        } else {
            assert_eq!(properties.as_object().unwrap().len(), 1, "{kind}");
        }
        assert!(PropertiesSite::from_address(
            &QualifiedAddress::parse(&format!("main:{kind}")).unwrap()
        )
        .is_none());
    }
}

#[test]
fn root_type_profiles_decode_the_existing_closed_type_vocabulary() {
    for kind in [
        "CommonAttribute",
        "SessionParameter",
        "FilterCriterion",
        "DefinedType",
    ] {
        let site = PropertiesSite::from_address(
            &QualifiedAddress::parse(&format!("main:{kind}.Existing")).unwrap(),
        )
        .unwrap_or_else(|| panic!("{kind} must have a Type profile"));
        for variant in [
            json!({"kind":"string", "length":17, "allowedLength":"variable"}),
            json!({"kind":"reference", "metadataPath":"Catalog.Items"}),
            json!({"kind":"definedType", "metadataPath":"DefinedType.Other"}),
        ] {
            crate::property_input::decode_properties(
                PropertiesOperation::Set,
                site,
                &json!({"values":{"Type":{"variants":[variant]}}}),
                "ops[3].args",
            )
            .unwrap_or_else(|error| panic!("{kind}: {error:?}"));
        }
    }
}

#[test]
fn root_type_profiles_refuse_unknown_read_only_and_invalid_qualified_values() {
    for kind in [
        "CommonAttribute",
        "SessionParameter",
        "FilterCriterion",
        "DefinedType",
    ] {
        let site = PropertiesSite::from_address(
            &QualifiedAddress::parse(&format!("main:{kind}.Existing")).unwrap(),
        )
        .unwrap();
        for values in [
            json!({"Type":null}),
            json!({"Type":"string(12)"}),
            json!({"Type":{"variants":[]}}),
            json!({"Type":{"variants":[{"kind":"future"}]}}),
            json!({"Type":{"variants":[{"kind":"typeId","typeId":"12345678-1234-4123-8123-123456789abc"}]}}),
            json!({"Type":{"variants":[{"kind":"characteristicTypes","metadataPath":"ChartOfCharacteristicTypes.Kinds"}]}}),
            json!({"Type":{"variants":[{"kind":"object","metadataPath":"Catalog.Items"}]}}),
            json!({"Type":{"variants":[{"kind":"reference","metadataPath":"Report.Items"}]}}),
            json!({"Type":{"variants":[{"kind":"reference","metadataPath":"other:Catalog.Items"}]}}),
            json!({"Type":{"variants":[{"kind":"definedType","metadataPath":"Catalog.Items"}]}}),
            json!({"Type":{"variants":[{"kind":"string","length":0,"allowedLength":"fixed"}]}}),
            json!({"Type":{"variants":[{"kind":"number","digits":2,"fraction":3,"sign":"any"}]}}),
            json!({"Type":{"variants":[{"kind":"boolean"}],"mutationCapability":"readOnly"}}),
            json!({"Type":{"variants":[{"kind":"boolean"}]},"type":{"variants":[{"kind":"boolean"}]}}),
        ] {
            let error = crate::property_input::decode_properties(
                PropertiesOperation::Set,
                site,
                &json!({"values":values}),
                "ops[3].args",
            )
            .expect_err(&format!("{kind}: {values}"));
            assert!(
                error.path.starts_with("ops[3].args.values"),
                "{kind}: {error:?}"
            );
        }
        if kind != "DefinedType" {
            for field in ["Comment", "Synonym", "Name", "Content"] {
                let values = json!({"Type":{"variants":[{"kind":"boolean"}]},field:"unrequested"});
                assert!(
                    crate::property_input::decode_properties(
                        PropertiesOperation::Set,
                        site,
                        &json!({"values":values}),
                        "ops[3].args"
                    )
                    .is_err(),
                    "{kind}: {field}"
                );
            }
        }
    }
}
