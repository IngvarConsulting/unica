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
    let input = json!({"at": "main:Document.First", "ops": [op], "dryRun": true});
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

    // 100 fits every mode; only mode 8.1 allows more.
    plan(&seam, add_fixed_string(100)).unwrap();
    plan(&fixture("Version8_1"), add_fixed_string(150)).unwrap();
}
