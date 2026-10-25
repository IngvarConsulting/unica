//! Канонические attribute.add, command.add и item.add пишут Form.xml в
//! профиле выгрузки 8.3.27 (#1996).

use super::actor_capacity_tests::direct_v5;
use super::*;
use unica_execution::invocation_store::ToolIdentity;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use unica_execution_contract::invocation::DomainResult;

const FORM: &str = "main:Catalog.Товары.Form.ФормаЭлемента";
const FORM_XML: &str = "src/Catalogs/Товары/Forms/ФормаЭлемента/Ext/Form.xml";

struct Fixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    runtime: V5CanonicalInvocationRuntime,
}

impl Fixture {
    /// The empty 8.3.27 configuration with a catalog and its object form.
    fn catalog_form() -> Self {
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
        fixture.publish(json!({"at": "main:Configuration", "ops": [
            {"op": "object.create", "args": {"values": {"kind": "Catalog", "name": "Товары"}}},
            {"op": "form.add", "args": {"at": "main:Catalog.Товары", "items": [{"name": "ФормаЭлемента", "type": "ObjectForm"}]}}
        ]}));
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
    fn publish(&self, args: Value) {
        let planned = self.call(ToolIdentity::Apply, args);
        assert!(planned.ok, "{planned:?}");
        let token = planned.data.as_ref().unwrap()["executionToken"].clone();
        let published = self.call(ToolIdentity::Apply, json!({"executionToken": token}));
        assert!(published.ok, "{published:?}");
    }
}

/// Attributes of every bare and qualified primitive type, a value table, a
/// command and elements of the form and its command bar.
fn form_content() -> Value {
    json!({"at": FORM, "ops": [{
        "op": "attribute.add",
        "args": {"items": [
            {"name": "Строка", "type": "String"},
            {"name": "Число", "type": "Number"},
            {"name": "Дата", "type": "Date"},
            {"name": "Флаг", "type": "Boolean"},
            {"name": "Код", "type": "string(10)"},
            {"name": "Сумма", "type": "decimal(15,2)"},
            {"name": "Строки", "type": "ValueTable", "columns": [
                {"name": "Текст", "type": "String"},
                {"name": "Количество", "type": "Number"}
            ]}
        ]}
    }, {
        "op": "command.add",
        "args": {"items": [{"name": "Заполнить", "title": "Заполнить"}]}
    }, {
        "op": "item.add",
        "args": {"items": [
            {"name": "ПолеСтрока", "type": "InputField", "path": "Строка"},
            {"name": "ПолеФлаг", "type": "CheckBox", "path": "Флаг"},
            {"name": "Группа", "type": "Group", "orientation": "Horizontal"},
            {"name": "ТаблицаСтроки", "type": "Table", "path": "Строки"}
        ]}
    }, {
        "op": "item.add",
        "args": {
            "at": format!("{FORM}.Item.ФормаКоманднаяПанель"),
            "items": [{"name": "КнопкаЗаполнить", "type": "Button", "command": "Заполнить"}]
        }
    }]})
}

/// The form canonical attribute.add, command.add and item.add write is the
/// form 1C 8.3.27 exports after loading it: bare types carry the platform's
/// default qualifiers and inserted lines take the form's CRLF.
#[test]
fn canonical_form_content_is_written_as_1c_exports_it() {
    let fixture = Fixture::catalog_form();
    fixture.publish(form_content());
    let written = std::fs::read(fixture.workspace.join(FORM_XML)).unwrap();
    // A bare Number takes 10 digits, as the configurator sets for a new
    // attribute and as a value table column has.
    let text = String::from_utf8_lossy(&written).replace("\r\n", "\n");
    let number = &text[text.find("<Attribute name=\"Число\"").unwrap()..];
    let number = &number[..number.find("</Attribute>").unwrap()];
    assert!(number.contains("<v8:Digits>10</v8:Digits>"), "{number}");
    let native = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/platform_8_3_27/form-native-profile/native-form.xml"),
    )
    .unwrap();
    assert!(
        written == native,
        "Form.xml differs from the 8.3.27 export:\n{}",
        String::from_utf8_lossy(&written)
    );
}
