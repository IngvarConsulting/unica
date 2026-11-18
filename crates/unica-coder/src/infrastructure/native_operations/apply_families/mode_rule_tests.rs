use super::{plan_hidden_v13_apply, tests::ApplySeamFixture};
use crate::domain::apply::ApplyRequest;
use serde_json::{json, Value};
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
use unica_source_contract::SourceSetKind;
use unica_workspace::test_support::tree_snapshot;

/// A configuration whose `CompatibilityMode` is `mode`.
fn fixture(mode: &str) -> ApplySeamFixture {
    let seam = ApplySeamFixture::with_kind(SourceSetKind::Configuration);
    std::fs::write(
        seam.source_dir().join("Configuration.xml"),
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><CompatibilityMode>{mode}</CompatibilityMode><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>First</Document><Document>Second</Document></ChildObjects></Configuration></MetaDataObject>"#
        ),
    )
    .unwrap();
    std::fs::write(
        seam.source_dir().join("Documents/First.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Document uuid="11111111-1111-4111-8111-111111111111">
		<Properties><Name>First</Name><Synonym/><Comment/><BasedOn/></Properties>
		<ChildObjects/>
	</Document>
</MetaDataObject>
"#,
    )
    .unwrap();
    seam
}

fn add_fixed_string(length: u32) -> Value {
    json!({"op": "attribute.add", "args": {
        "at": "main:Document.First",
        "items": [{
            "name": "Code",
            "type": {"variants": [{"kind": "string", "length": length, "allowedLength": "fixed"}]}
        }]
    }})
}

fn plan(seam: &ApplySeamFixture, op: Value) -> Result<(), ApplyPlanError> {
    let at = op["args"]["at"].clone();
    let input = json!({"at": at, "ops": [op], "dryRun": true});
    let request = ApplyRequest::parse(input.as_object().unwrap(), &["main"]).unwrap();
    plan_hidden_v13_apply(&request, &seam.binding, &seam.admission()).map(|_| ())
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_fixed_string_longer_than_the_mode_allows_is_refused_before_publishing() {
    let seam = fixture("Version8_3_24");
    let before = tree_snapshot(&seam.source_dir());
    let error = plan(&seam, add_fixed_string(150))
        .expect_err("a fixed-length string of 150 is longer than mode 8.3.24 allows");
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{error:?}");
    let message = error.to_string();
    assert!(
        message.contains("150") && message.contains("Version8_3_24"),
        "{message}"
    );
    assert_eq!(tree_snapshot(&seam.source_dir()), before);

    // 100 fits every mode Unica edits.
    plan(&seam, add_fixed_string(100)).unwrap();
}

fn set_mode(property: &str, mode: &str) -> Value {
    json!({"op": "props.set", "args": {"at": "main:Configuration", "values": {property: mode}}})
}

fn refusal_code(error: &ApplyPlanError) -> &str {
    error
        .diagnostics()
        .first()
        .and_then(|d| d["code"].as_str())
        .unwrap_or_else(|| panic!("{error:?}"))
}

/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn a_set_older_than_mode_8_3_8_is_read_only_but_may_raise_its_mode() {
    // Another edit in mode 8.3.7 is refused and names the mode, the minimum
    // and that the mode can be raised; nothing is written.
    let refused = fixture("Version8_3_7");
    let before = tree_snapshot(&refused.source_dir());
    let error = plan(&refused, add_fixed_string(10)).expect_err("mode 8.3.7 is read-only");
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource, "{error:?}");
    let diagnostic = &error.diagnostics()[0];
    assert_eq!(diagnostic["code"], "compatibilityModeUnsupported");
    assert_eq!(diagnostic["mode"], "Version8_3_7");
    assert_eq!(diagnostic["minimumMode"], "Version8_3_8");
    assert_eq!(diagnostic["property"], "CompatibilityMode");
    assert!(error
        .to_string()
        .contains("set CompatibilityMode to Version8_3_8 or newer"));
    assert_eq!(tree_snapshot(&refused.source_dir()), before);

    // Raising the mode to 8.3.8 or newer is the one edit allowed.
    for raised in ["Version8_3_8", "Version8_3_27"] {
        plan(
            &fixture("Version8_3_7"),
            set_mode("CompatibilityMode", raised),
        )
        .unwrap_or_else(|error| panic!("{raised}: {error:?}"));
    }
    // Raising it to a mode still older than 8.3.8 is refused.
    let error = plan(
        &fixture("Version8_3_7"),
        set_mode("CompatibilityMode", "Version8_3_6"),
    )
    .expect_err("8.3.6 is still read-only");
    assert_eq!(refusal_code(&error), "compatibilityModeUnsupported");

    // From 8.3.8 on every edit is allowed.
    plan(&fixture("Version8_3_8"), add_fixed_string(10)).unwrap();
}

/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn an_extension_is_read_only_by_its_own_mode() {
    let extension = |mode: &str| {
        let seam = ApplySeamFixture::with_kind(SourceSetKind::Extension);
        std::fs::write(
            seam.source_dir().join("Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>Ext</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose><CompatibilityMode>Version8_3_27</CompatibilityMode><ConfigurationExtensionCompatibilityMode>{mode}</ConfigurationExtensionCompatibilityMode><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>First</Document><Document>Second</Document></ChildObjects></Configuration></MetaDataObject>"#
            ),
        )
        .unwrap();
        seam
    };
    let edit = json!({"op": "props.set", "args": {"at": "main:Document.First", "values": {"Comment": "x"}}});
    let error = plan(&extension("Version8_3_7"), edit.clone())
        .expect_err("the extension's own mode 8.3.7 is read-only");
    assert_eq!(
        error.diagnostics()[0]["code"],
        "compatibilityModeUnsupported"
    );
    plan(&extension("Version8_3_8"), edit).unwrap();

    // The extension raises its own mode; another edit of its root does not.
    plan(
        &extension("Version8_3_7"),
        set_mode("ConfigurationExtensionCompatibilityMode", "Version8_3_8"),
    )
    .unwrap();
    let error = plan(&extension("Version8_3_7"), set_mode("Comment", "x"))
        .expect_err("only raising its own mode is allowed");
    assert_eq!(refusal_code(&error), "compatibilityModeUnsupported");
}
