use super::*;

/// The Can-based name invariant of #2160, applied to one corresponding
/// payload shape, not a union of unrelated operation schemas (#2183).
fn mismatched_names(props: &Value, writable: &Value, observed: &[&str]) -> Vec<String> {
    let accepted = writable.as_object().expect("published property schema");
    props
        .as_object()
        .expect("view props")
        .keys()
        .filter(|key| !accepted.contains_key(*key) && !observed.contains(&key.as_str()))
        .cloned()
        .collect()
}

#[test]
fn dcs_parameter_and_field_props_use_the_names_their_can_accepts() {
    let fixture = Fixture::new("workspace-dcs");
    let xml = fixture
        .source
        .join("cf/Reports/F05Report/Templates/F05Schema/Ext/Template.xml");
    let text = std::fs::read_to_string(&xml).unwrap().replace("</DataCompositionSchema>",
        "<parameter><name>Period</name><expression>1</expression><useRestriction>true</useRestriction><availableAsField>false</availableAsField></parameter></DataCompositionSchema>");
    std::fs::write(xml, text).unwrap();
    let runtime = runtime();
    let schema = "cf:Report.F05Report.Template.F05Schema";
    for (suffix, op) in [
        ("Parameter.Period", "parameter.set"),
        ("DataSet.F05Data.Field.Amount", "field.set"),
    ] {
        let at = format!("{schema}.{suffix}");
        let data = view(&runtime, &fixture, &at);
        let can = contract(&runtime, &fixture, &at, op);
        assert!(
            mismatched_names(
                &data["props"],
                &can["argsSchema"]["properties"]["values"]["properties"],
                &[]
            )
            .is_empty(),
            "{at}: read properties must use the corresponding Can names"
        );
    }
    let data = view(&runtime, &fixture, &format!("{schema}.Parameter.Period"));
    assert_eq!(data["props"]["useRestriction"], true);
    assert!(data["props"].get("restricted").is_none());
}

use crate::infrastructure::logical_tree::{route_logical_address, LogicalReader};
use crate::infrastructure::v13_read_projection::structured_properties::{
    self, PropertyRole, WriteTarget,
};
use unica_address::address::QualifiedAddress;
use unica_bsl::profile::PlatformProfile;

fn profile_mismatches(
    at: &str,
    props: &Value,
    mut accepted: impl FnMut(&structured_properties::PropertySpec) -> Value,
) -> Vec<String> {
    let address = QualifiedAddress::parse(at).unwrap();
    let reader = if props["format"] == "DataCompositionSchema" {
        LogicalReader::Dcs
    } else {
        route_logical_address(&address, PlatformProfile::v8_3_27())
            .unwrap()
            .reader()
    };
    let kind = address.segments().last().unwrap().kind();
    props
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, _)| {
            let Some(spec) = structured_properties::property(reader, kind, &address, props, name)
            else {
                return Some(name.clone());
            };
            match spec.role {
                PropertyRole::WriteName { .. } => {
                    let shape = accepted(&spec);
                    (!shape
                        .as_object()
                        .is_some_and(|fields| fields.contains_key(name)))
                    .then(|| name.clone())
                }
                PropertyRole::Observed(reason) | PropertyRole::Derived(reason) => {
                    reason.trim().is_empty().then(|| name.clone())
                }
            }
        })
        .collect()
}

fn assert_read_profile(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture, data: &Value) {
    let at = data["at"].as_str().expect("addressable node");
    let address = QualifiedAddress::parse(at).unwrap();
    let mismatches = profile_mismatches(at, &data["props"], |spec| {
        let PropertyRole::WriteName {
            operation,
            target,
            payload,
        } = spec.role
        else {
            unreachable!()
        };
        let target = match target {
            WriteTarget::SelfNode => address.clone(),
            WriteTarget::Parent => address.prefix(address.segments().len() - 1),
            WriteTarget::Package => address.prefix(1),
        };
        let can = contract(runtime, fixture, &target.to_string(), operation);
        can["argsSchema"]
            .pointer(payload)
            .cloned()
            .unwrap_or(Value::Null)
    });
    assert!(
        mismatches.is_empty(),
        "{at}: unclassified or mismatched property names: {mismatches:?}"
    );
}

#[test]
fn dcs_and_xdto_final_props_have_explicit_roles_and_match_corresponding_can() {
    let fixture = Fixture::new("workspace-structured-property-names");
    let runtime = runtime();
    let mut pending = vec![
        "cf:Report.F05Report.Template.F05Schema".to_string(),
        "cf:XDTOPackage.Contracts".to_string(),
    ];
    let mut visited = BTreeSet::new();
    let mut covered = BTreeSet::new();
    while let Some(at) = pending.pop() {
        if !visited.insert(at.clone()) {
            continue;
        }
        let data = view(&runtime, &fixture, &at);
        assert_read_profile(&runtime, &fixture, &data);
        covered.insert(data["kind"].as_str().unwrap_or("").to_string());
        for branch in data["branches"].as_array().into_iter().flatten() {
            pending.push(branch["at"].as_str().unwrap().to_string());
        }
        for node in data["items"].as_array().into_iter().flatten() {
            if let Some(at) = node["at"].as_str() {
                assert_read_profile(&runtime, &fixture, node);
                pending.push(at.to_string());
            }
        }
    }
    for kind in [
        "Template",
        "DataSource",
        "DataSet",
        "Field",
        "Parameter",
        "CalculatedField",
        "Total",
        "Setting",
        "Item",
        "XDTOPackage",
        "Namespace",
        "Type",
        "Property",
    ] {
        assert!(covered.contains(kind), "missing guard coverage: {kind}");
    }
    let borrowed = Fixture::new("workspace-borrowed");
    let inherited = view(&runtime, &borrowed, "ext:XDTOPackage.ПакетОбмена");
    assert_eq!(inherited["props"]["content"], "inherited");
    assert_read_profile(&runtime, &borrowed, &inherited);
    let at = "cf:XDTOPackage.Contracts.Type.Envelope.Property.Limited";
    let anonymous = view(&runtime, &fixture, at);
    assert_eq!(anonymous["props"]["anonymousType"], "ValueType");
    assert_eq!(anonymous["props"]["facets"]["maxLength"], 10);
    let package = "cf:XDTOPackage.Contracts";
    let properties = view(
        &runtime,
        &fixture,
        &format!("{package}.Type.Envelope.Property"),
    );
    let reference = properties["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row.get("ref").is_some())
        .expect("global-property reference remains observable");
    assert_eq!(reference["ref"]["raw"], "tns:Global");
    assert_eq!(reference["ref"]["namespace"], "urn:unica:property-names");
    assert!(
        reference.get("at").is_none(),
        "a reference row does not acquire a fabricated address"
    );
    assert_eq!(
        reference["bounds"],
        json!({"lower":"1","upper":"1","lowerExplicit":false,"upperExplicit":false,"unbounded":false})
    );
    assert_eq!(
        view(&runtime, &fixture, &format!("{package}.Type.Text"))["props"]["facets"]["maxLength"],
        20
    );
}

#[test]
fn profile_guard_detects_unknown_props_changed_operands_and_wrong_anonymous_context() {
    let fixture = Fixture::new("workspace-structured-property-names");
    let runtime = runtime();
    let at = "cf:Report.F05Report.Template.F05Schema.Parameter.Period";
    for key in ["restricted", "sourceField", "minOccurs", "maxOccurs"] {
        assert_eq!(
            profile_mismatches(at, &json!({key:true}), |_| unreachable!()),
            vec![key]
        );
    }

    let data = view(&runtime, &fixture, at);
    let mut can = contract(&runtime, &fixture, at, "parameter.set");
    let shape = can["argsSchema"]["properties"]["values"]["properties"]
        .as_object_mut()
        .unwrap();
    let value = shape.remove("useRestriction").unwrap();
    shape.insert("restricted".into(), value);
    assert_eq!(
        profile_mismatches(at, &data["props"], |_| json!(shape)),
        vec!["useRestriction"]
    );
    assert_eq!(
        profile_mismatches(
            "cf:XDTOPackage.Contracts.Type.Envelope.Property.Probe",
            &json!({"futureProperty":true}),
            |_| unreachable!()
        ),
        vec!["futureProperty"]
    );
    assert_eq!(
        profile_mismatches(
            "cf:XDTOPackage.Contracts.Type.Envelope.Property.Probe",
            &json!({"facets":{},"anonymousType":"ObjectType"}),
            |_| unreachable!()
        ),
        vec!["facets"]
    );
}
