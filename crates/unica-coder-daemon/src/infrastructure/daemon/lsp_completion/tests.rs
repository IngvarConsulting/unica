use super::*;

#[test]
fn an_object_descriptor_gives_its_attributes_and_sections_with_columns() {
    let descriptor = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="1"><Properties><Name>Items</Name></Properties><ChildObjects><Attribute uuid="2"><Properties><Name>Article</Name></Properties></Attribute><TabularSection uuid="3"><Properties><Name>Barcodes</Name></Properties><ChildObjects><Attribute uuid="4"><Properties><Name>Barcode</Name></Properties></Attribute></ChildObjects></TabularSection></ChildObjects></Catalog></MetaDataObject>"#;
    assert_eq!(
        object_members(descriptor.as_bytes()),
        Some(ObjectMembers {
            attributes: vec!["Article".to_owned()],
            tabular_sections: vec![("Barcodes".to_owned(), vec!["Barcode".to_owned()])],
        })
    );
}

#[test]
fn an_object_or_form_module_names_its_owner_and_another_module_none() {
    assert_eq!(
        module_owner("main:Catalog.Items.Module.Object"),
        Some(ModuleOwner {
            kind: "Catalog".to_owned(),
            name: "Items".to_owned(),
            form: false,
        })
    );
    assert!(
        module_owner("main:Catalog.Items.Form.Item.Module.Form").is_some_and(|owner| owner.form)
    );
    assert_eq!(module_owner("main:CommonModule.Main"), None);
}
