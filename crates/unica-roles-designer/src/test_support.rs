//! Original role writer test fixtures, without source or publication authority.
use crate::writer::*;
use roxmltree::Document;
/// В фикстурах тестов документ целиком принадлежит писателю, поэтому
/// булевость требуется от каждого права: строгость прежнего флага
/// сохраняется, а продуктовый контракт остаётся адресным.
pub fn validate_every_role_value(text: &str) -> Result<(), String> {
    let pairs = match Document::parse(text) {
        Ok(document) => {
            direct_role_children(document.root_element(), "object", ROLE_RIGHTS_NAMESPACE)
                .into_iter()
                .flat_map(|object| {
                    let object_name = direct_role_children(object, "name", ROLE_RIGHTS_NAMESPACE)
                        .first()
                        .map(|node| role_text_content(*node))
                        .unwrap_or_default();
                    direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE)
                        .into_iter()
                        .map(move |right| {
                            let right_name =
                                direct_role_children(right, "name", ROLE_RIGHTS_NAMESPACE)
                                    .first()
                                    .map(|node| role_text_content(*node))
                                    .unwrap_or_default();
                            (object_name.clone(), right_name)
                        })
                })
                .collect::<Vec<_>>()
        }
        Err(_) => Vec::new(),
    };
    validate_role_rights_document(text, RoleValueScope::WrittenRights(&pairs))
}
pub fn operation(object_name: &str, right: &str, value: bool) -> unica_roles::RoleEditOperation {
    unica_roles::RoleEditOperation {
        object_name: object_name.to_string(),
        right: right.to_string(),
        value,
    }
}
pub fn rights_xml(eol: &str, self_closing: &str) -> String {
    [
        r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" xmlns:r="http://v8.1c.ru/8.2/roles" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="Rights" version="2.20">"#,
        "\t<setForNewObjects>false</setForNewObjects>",
        "\t<setForAttributesByDefault>true</setForAttributesByDefault>",
        "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
        "\t<object>",
        "\t\t<name>Catalog.Demo</name>",
        "\t\t<future><name>Delete</name><value>false</value></future>",
        "\t\t<right>",
        "\t\t\t<name>Delete</name>",
        &format!("\t\t\t{self_closing}"),
        "\t\t\t<restrictionByCondition><condition>WHERE false</condition></restrictionByCondition>",
        "\t\t</right>",
        "\t\t<right><name>FuturePlatformRight</name><value>true</value><future/></right>",
        "\t</object>",
        "\t<object>",
        "\t\t<name>DataProcessor.Worker</name>",
        "\t\t<right><name>Use</name><value>true</value></right>",
        "\t\t<right><name>View</name><value>true</value></right>",
        "\t</object>",
        "\t<restrictionTemplate><name>Keep</name><condition>TRUE</condition></restrictionTemplate>",
        "</Rights>",
    ]
    .join(eol)
}
