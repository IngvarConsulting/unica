use super::*;

#[test]
fn implementation_shape_uses_typed_owner_leaf_and_the_selected_catalog_record() {
    let form_xml = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
            <AutoCommandBar name="Bar" id="-1"/>
            <ChildItems>
                <InputField name="Field" id="1"/>
                <Table name="Goods" id="2"><ChildItems>
                    <InputField name="Quantity" id="3"/>
                </ChildItems></Table>
            </ChildItems>
            <Commands><Command name="Заполнить"/></Commands>
        </Form>"#;
    let cases = [
            (
                "main:Catalog.Products.Form.Main.Event.OnOpen",
                EventImplementationOwner::Form,
                "ПриОткрытии",
                "Процедура ПриОткрытии(Отказ)",
                "ОбработчикиСобытийФормы",
            ),
            (
                "main:Catalog.Products.Form.Main.Item.Field.Event.OnChange",
                EventImplementationOwner::Element,
                "FieldПриИзменении",
                "Процедура FieldПриИзменении(Элемент)",
                "ОбработчикиСобытийЭлементовФормы",
            ),
            (
                "main:Catalog.Products.Form.Main.Item.Goods.Event.BeforeAddRow",
                EventImplementationOwner::Table,
                "GoodsПередНачаломДобавления",
                "Процедура GoodsПередНачаломДобавления(Элемент, Отказ, Копирование, Родитель, ЭтоГруппа, Параметр)",
                "ОбработчикиСобытийЭлементовФормы",
            ),
            (
                "main:Catalog.Products.Form.Main.Item.Goods.Item.Quantity.Event.OnChange",
                EventImplementationOwner::Column,
                "QuantityПриИзменении",
                "Процедура QuantityПриИзменении(Элемент)",
                "ОбработчикиСобытийЭлементовФормы",
            ),
            (
                "main:Catalog.Products.Form.Main.Command.Заполнить.Event.Execute",
                EventImplementationOwner::Command,
                "ЗаполнитьОбработкаКоманды",
                "Процедура ЗаполнитьОбработкаКоманды(Команда)",
                "ОбработчикиКомандФормы",
            ),
        ];
    for (at, owner, handler, signature, region) in cases {
        let at = QualifiedAddress::parse(at).unwrap();
        let source = resolve_property_event_source(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &at,
            form_xml,
        )
        .unwrap();
        let shape = project_property_event_implementation(&source, form_xml, None).unwrap();
        assert_eq!(shape.owner, owner, "{at}");
        assert_eq!(shape.handler, handler, "{at}");
        assert_eq!(
            shape.implementation_at,
            format!("{}.Method.{handler}", source.module_at),
            "{at}"
        );
        assert_eq!(shape.signature, signature, "{at}");
        assert_eq!(shape.region, region, "{at}");
        assert_eq!(shape.method_kind, MethodKind::Procedure, "{at}");
    }
}
