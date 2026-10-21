//! Original XDTO test input fixtures, without source authority.
use serde_json::{Map, Value};
pub const PACKAGE: &str = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" targetNamespace="urn:test">
	<objectType name="ЛюбаяСсылка">
		<property name="СсылкаНаОбъект">
			<typeDef xsi:type="ObjectType">
			</typeDef>
		</property>
	</objectType>
	<objectType name="СоставнойЛюбойОбъект"/>
</package>"#;
pub fn args(entries: &[(&str, Value)]) -> Map<String, Value> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}
