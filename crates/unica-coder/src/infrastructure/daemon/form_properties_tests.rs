//! `props.set` на форме пишет свойства корня `Form.xml` (#1984).

use super::actor_capacity_tests::direct_v5;
use super::*;
use unica_execution::invocation_store::ToolIdentity;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use unica_execution_contract::invocation::DomainResult;

struct Fixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    runtime: V5CanonicalInvocationRuntime,
}

impl Fixture {
    /// The empty 8.3.27 configuration with the given objects and forms made
    /// by canonical apply.
    fn new(objects: &[(&str, &str)], forms: &[(&str, &str, &str)]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            unica_workspace::test_support::canonical_path(root.path()).join("workspace");
        let image = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/platform_8_3_27/empty_configuration");
        for (relative, bytes) in unica_workspace::test_support::tree_snapshot(&image) {
            let path = workspace.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let fixture = Self {
            _root: root,
            workspace,
            runtime: V5CanonicalInvocationRuntime::new(
                Arc::new(
                    crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
                ),
                Arc::new(TokioClock),
            ),
        };
        let mut ops = objects
            .iter()
            .map(|(kind, name)| {
                json!({"op":"object.create","args":{"values":{"kind":kind,"name":name}}})
            })
            .collect::<Vec<_>>();
        for (owner, form, kind) in forms {
            ops.push(json!({"op":"form.add","args":{"at":format!("main:{owner}"),"items":[{"name":form,"type":kind}]}}));
        }
        fixture.publish(json!({"at":"main:Configuration","ops":ops}));
        fixture
    }

    fn call(&self, tool: ToolIdentity, args: Value) -> DomainResult {
        direct_v5(
            &self.runtime,
            InvocationRequest::new(tool, args, self.workspace.to_string_lossy(), 7_000).unwrap(),
        )
        .unwrap()
    }

    /// Preview, then execution by the saved token.
    fn publish(&self, args: Value) -> DomainResult {
        let planned = self.call(ToolIdentity::Apply, args);
        assert!(planned.ok, "{planned:?}");
        let token = planned.data.as_ref().unwrap()["executionToken"].clone();
        let published = self.call(ToolIdentity::Apply, json!({"executionToken": token}));
        assert!(published.ok, "{published:?}");
        published
    }
}

const FORM: &str = "main:Catalog.Товары.Form.ФормаЭлемента";
const FORM_XML: &str = "src/Catalogs/Товары/Forms/ФормаЭлемента/Ext/Form.xml";

fn catalog_form() -> Fixture {
    Fixture::new(
        &[("Catalog", "Товары"), ("Document", "Заказ")],
        &[
            ("Catalog.Товары", "ФормаЭлемента", "ObjectForm"),
            ("Document.Заказ", "ФормаДокумента", "ObjectForm"),
        ],
    )
}

fn props_set(at: &str, values: Value) -> Value {
    json!({"at": at, "ops": [{"op": "props.set", "args": {"values": values}}]})
}

/// Every property `props.set` writes, none at its platform default.
fn every_property() -> Value {
    json!({
        "Title": "Товар", "Width": 61, "Height": 17,
        "WindowOpeningMode": "LockWholeInterface", "EnterKeyBehavior": "DefaultButton",
        "AutoSaveDataInSettings": "Use", "SaveDataInSettings": "UseList",
        "SaveWindowSettings": false, "AutoTitle": false, "AutoURL": false,
        "Group": "Horizontal", "ChildrenAlign": "None", "HorizontalSpacing": "Double",
        "VerticalSpacing": "Half", "HorizontalAlign": "Center", "VerticalAlign": "Bottom",
        "ChildItemsWidth": "LeftWide", "AutoFillCheck": false, "Customizable": false,
        "Enabled": false, "ReadOnly": true, "CommandBarLocation": "Bottom",
        "VerticalScroll": "useWithoutStretch", "ScalingMode": "Compact", "Scale": 98,
        "ConversationsRepresentation": "DontShow", "ShowTitle": false, "ShowCloseButton": false,
        "CollapseItemsByImportanceVariant": "DontUse", "UseForFoldersAndItems": "Folders"
    })
}

/// The same properties at the values 1C does not export.
fn every_default() -> Value {
    json!({
        "Title": "", "Width": 0, "Height": 0,
        "WindowOpeningMode": "Independent", "EnterKeyBehavior": "ControlNavigation",
        "AutoSaveDataInSettings": "DontUse", "SaveDataInSettings": "DontUse",
        "SaveWindowSettings": true, "AutoTitle": true, "AutoURL": true,
        "Group": "Vertical", "ChildrenAlign": "Auto", "HorizontalSpacing": "Auto",
        "VerticalSpacing": "Auto", "HorizontalAlign": "Auto", "VerticalAlign": "Auto",
        "ChildItemsWidth": "Auto", "AutoFillCheck": true, "Customizable": true,
        "Enabled": true, "ReadOnly": false, "CommandBarLocation": "Auto",
        "VerticalScroll": "auto", "ScalingMode": "Auto", "Scale": 100,
        "ConversationsRepresentation": "Auto", "ShowTitle": true, "ShowCloseButton": true,
        "CollapseItemsByImportanceVariant": "Auto", "UseForFoldersAndItems": "Items"
    })
}

/// The form written by props.set is the form 1C 8.3.27 exports after
/// loading it; platform defaults are not written, so setting them back
/// restores the form `form.add` made.
#[test]
fn canonical_props_set_writes_form_root_properties_as_1c_exports_them() {
    let fixture = catalog_form();
    let path = fixture.workspace.join(FORM_XML);
    let created = std::fs::read(&path).unwrap();

    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let planned = fixture.call(ToolIdentity::Apply, props_set(FORM, every_property()));
    assert!(planned.ok, "{planned:?}");
    assert_eq!(
        planned.changed,
        vec![json!({"at": FORM, "event": "FormChanged"})]
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before,
        "the preview writes nothing"
    );
    let token = planned.data.as_ref().unwrap()["executionToken"].clone();
    let published = fixture.call(ToolIdentity::Apply, json!({"executionToken": token}));
    assert!(published.ok, "{published:?}");
    let native = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/platform_8_3_27/form-root-properties/native-form.xml"),
    )
    .unwrap();
    assert!(
        std::fs::read(&path).unwrap() == native,
        "Form.xml differs from the 8.3.27 export:\n{}",
        String::from_utf8_lossy(&std::fs::read(&path).unwrap())
    );

    // view reads back what props.set wrote, under the same names.
    let viewed = fixture.call(ToolIdentity::View, json!({"at": FORM}));
    assert!(viewed.ok, "{viewed:?}");
    let viewed = viewed.data.unwrap();
    assert_eq!(viewed["title"], "Товар");
    let mut written = every_property();
    written.as_object_mut().unwrap().remove("Title");
    assert_eq!(viewed["props"]["properties"], written, "{viewed}");

    fixture.publish(props_set(FORM, every_default()));
    assert!(
        std::fs::read(&path).unwrap() == created,
        "defaults restore the form.add bytes:\n{}",
        String::from_utf8_lossy(&std::fs::read(&path).unwrap())
    );
}

#[test]
fn canonical_props_set_writes_a_multilingual_title_and_document_properties() {
    let fixture = catalog_form();
    fixture.publish(props_set(
        FORM,
        json!({"Title": {"ru": "Товар", "en": "Product"}, "AutoTitle": false}),
    ));
    let text = String::from_utf8(std::fs::read(fixture.workspace.join(FORM_XML)).unwrap())
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        text.contains(concat!(
            "\t<Title>\n",
            "\t\t<v8:item>\n\t\t\t<v8:lang>ru</v8:lang>\n\t\t\t<v8:content>Товар</v8:content>\n\t\t</v8:item>\n",
            "\t\t<v8:item>\n\t\t\t<v8:lang>en</v8:lang>\n\t\t\t<v8:content>Product</v8:content>\n\t\t</v8:item>\n",
            "\t</Title>\n",
            "\t<AutoTitle>false</AutoTitle>\n",
        )),
        "{text}"
    );

    // A document form takes the posting properties; 1C exports them always.
    let document = "main:Document.Заказ.Form.ФормаДокумента";
    fixture.publish(props_set(
        document,
        json!({"AutoTime": "DontUse", "UsePostingMode": "Regular", "RepostOnWrite": false}),
    ));
    let text = String::from_utf8(
        std::fs::read(
            fixture
                .workspace
                .join("src/Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml"),
        )
        .unwrap(),
    )
    .unwrap()
    .replace("\r\n", "\n");
    assert!(
        text.contains(concat!(
            "\t<AutoTime>DontUse</AutoTime>\n",
            "\t<UsePostingMode>Regular</UsePostingMode>\n",
            "\t<RepostOnWrite>false</RepostOnWrite>\n",
            "\t<AutoCommandBar",
        )),
        "{text}"
    );
}

/// A property the form does not take refuses the preview with bad_value and
/// writes nothing: an unknown name, a reference property, a property of
/// another kind of form and a value outside its type.
#[test]
fn canonical_props_set_refuses_what_the_form_does_not_take() {
    let fixture = catalog_form();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    for (values, path, part) in [
        (
            json!({"Colour": "Red"}),
            "values.Colour",
            "unknown argument",
        ),
        (
            json!({"ReportResult": "Результат"}),
            "values.ReportResult",
            "unknown argument",
        ),
        (json!({"Width": -1}), "values.Width", ""),
        (json!({"Group": "Diagonal"}), "values.Group", ""),
        (json!({"UsePostingMode": "Regular"}), "", "a document form"),
        (
            json!({"Title": "Товар", "ReportFormType": "Main"}),
            "",
            "a report form",
        ),
    ] {
        let refused = fixture.call(ToolIdentity::Apply, props_set(FORM, values.clone()));
        assert!(!refused.ok, "{values}: {refused:?}");
        assert_eq!(refused.diagnostics.len(), 1, "{values}: {refused:?}");
        assert_eq!(
            refused.diagnostics[0]["code"], "bad_value",
            "{values}: {refused:?}"
        );
        let message = refused.diagnostics[0]["message"].as_str().unwrap();
        assert!(
            message.contains(path) && message.contains(part),
            "{values}: {message}"
        );
        assert!(refused.changed.is_empty(), "{values}: {refused:?}");
    }
    // The argument refusal leads to the operation contract at the form.
    let refused = fixture.call(
        ToolIdentity::Apply,
        props_set(FORM, json!({"Colour": "Red"})),
    );
    assert!(
        refused
            .next
            .iter()
            .any(|next| next["args"]["at"] == format!("{FORM}.Can.props.set")),
        "{refused:?}"
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );

    // At the owner props.set keeps writing its descriptor.
    fixture.publish(props_set(
        "main:Catalog.Товары",
        json!({"Comment": "Справочник"}),
    ));
}

/// The document form `form.add` makes is the form 1C 8.3.27 exports after
/// loading it: the platform always writes the posting properties (#1994).
#[test]
fn canonical_form_add_writes_a_document_form_as_1c_exports_it() {
    let fixture = catalog_form();
    let native = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/platform_8_3_27/form-root-properties/native-document-form.xml",
    ))
    .unwrap();
    let written = std::fs::read(
        fixture
            .workspace
            .join("src/Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml"),
    )
    .unwrap();
    assert!(
        written == native,
        "Form.xml differs from the 8.3.27 export:\n{}",
        String::from_utf8_lossy(&written)
    );
}
