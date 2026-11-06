use super::*;
fn fixture(value:&str)->MetadataFixture {
    let config=RU_CONFIGURATION_XML.replace("<Configuration>","<Configuration uuid=\"16970000-0000-4000-8000-000000000010\">")
        .replace("</ChildObjects>","<Report>Probe</Report><SettingsStorage>StorageA</SettingsStorage><SettingsStorage>StorageB</SettingsStorage></ChildObjects>");
    let descriptor=|kind:&str,name:&str,field:&str|format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><{kind} uuid=\"16970000-0000-4000-8000-000000000011\"><Properties><Name>{name}</Name><Comment/>{field}</Properties><ChildObjects/></{kind}></MetaDataObject>");
    let report=descriptor("Report","Probe",&format!("<VariantsStorage>{value}</VariantsStorage><SettingsStorage/>"));
    MetadataFixture::with_files(&[("Configuration.xml",&config),("Documents/Order.xml",ORDER_XML),("Reports/Probe.xml",&report),("SettingsStorages/StorageA.xml",&descriptor("SettingsStorage","StorageA","")),("SettingsStorages/StorageB.xml",&descriptor("SettingsStorage","StorageB",""))])
}
#[test]
fn report_variant_storage_final_overlay_validates_noop_references_after_later_changes() {
    for case in ["storage_missing","storage_wrong_name","storage_unregistered","report_missing","report_removed"] {
        let fixture=fixture("SettingsStorage.StorageA");let before=unica_workspace::test_support::tree_snapshot(fixture.binding.source_root());let admission=fixture.admission();
        let authority=admission.metadata_planning_authority(&fixture.binding).unwrap();
        let operations=[fixture.parse("props.set",json!({"at":"main:Report.Probe","values":{"VariantsStorage":"SettingsStorage.StorageA"}}),0)];
        let (mut state,_)=plan_metadata_batch_owned_for_test(admission.staged_state().unwrap(),authority,&operations).unwrap();
        assert!(state.planned_changes().is_empty(),"same target is an actual no-op");
        unica_metadata_designer::report_variants_storage::validate_final(&mut state.proposal(),"main",SourceSetKind::Configuration,operations.iter().map(|op|op.operation())).unwrap();
        match case {
            "storage_missing"=>{let p=Path::new("SettingsStorages/StorageA.xml");let bytes=state.read(p).unwrap().unwrap();state.remove(p,&bytes).unwrap();},
            "storage_wrong_name"=>{let p=Path::new("SettingsStorages/StorageA.xml");let bytes=state.read(p).unwrap().unwrap();let text=String::from_utf8(bytes.clone()).unwrap().replace("<Name>StorageA</Name>","<Name>Other</Name>");state.replace(p,&bytes,text.into_bytes()).unwrap();},
            "storage_unregistered"=>{let p=Path::new("Configuration.xml");let bytes=state.read(p).unwrap().unwrap();let text=String::from_utf8(bytes.clone()).unwrap().replace("<SettingsStorage>StorageA</SettingsStorage>","");state.replace(p,&bytes,text.into_bytes()).unwrap();},
            "report_missing"|"report_removed"=>{let p=Path::new("Reports/Probe.xml");let bytes=state.read(p).unwrap().unwrap();state.remove(p,&bytes).unwrap();if case=="report_removed"{let p=Path::new("Configuration.xml");let bytes=state.read(p).unwrap().unwrap();let text=String::from_utf8(bytes.clone()).unwrap().replace("<Report>Probe</Report>","");state.replace(p,&bytes,text.into_bytes()).unwrap();}},
            _=>unreachable!(),
        }
        let result=unica_metadata_designer::report_variants_storage::validate_final(&mut state.proposal(),"main",SourceSetKind::Configuration,operations.iter().map(|op|op.operation()));
        assert_eq!(result.is_ok(),case=="report_removed","{case}: {result:?}");
        assert_eq!(unica_workspace::test_support::tree_snapshot(fixture.binding.source_root()),before,"staging/final validation never publishes partial images");
    }
}
#[test]
fn report_variant_storage_final_overlay_checks_the_final_reference_not_intermediate_targets() {
    let fixture=fixture("SettingsStorage.StorageA");let admission=fixture.admission();let authority=admission.metadata_planning_authority(&fixture.binding).unwrap();
    let operations=["StorageA","StorageB"].into_iter().enumerate().map(|(i,name)|fixture.parse("props.set",json!({"at":"main:Report.Probe","values":{"VariantsStorage":format!("SettingsStorage.{name}")}}),i)).collect::<Vec<_>>();
    let (mut state,_)=plan_metadata_batch_owned_for_test(admission.staged_state().unwrap(),authority,&operations).unwrap();
    let p=Path::new("SettingsStorages/StorageA.xml");let bytes=state.read(p).unwrap().unwrap();state.remove(p,&bytes).unwrap();
    unica_metadata_designer::report_variants_storage::validate_final(&mut state.proposal(),"main",SourceSetKind::Configuration,operations.iter().map(|op|op.operation())).unwrap();
    let p=Path::new("SettingsStorages/StorageB.xml");let bytes=state.read(p).unwrap().unwrap();state.remove(p,&bytes).unwrap();
    assert!(unica_metadata_designer::report_variants_storage::validate_final(&mut state.proposal(),"main",SourceSetKind::Configuration,operations.iter().map(|op|op.operation())).is_err());
}
#[test]
fn report_variant_storage_does_not_add_target_checks_to_an_unrelated_comment_edit() {
    let fixture=fixture("SettingsStorage.AlreadyMissing");let admission=fixture.admission();let authority=admission.metadata_planning_authority(&fixture.binding).unwrap();
    let operations=[fixture.parse("props.set",json!({"at":"main:Report.Probe","values":{"Comment":"independent"}}),0)];
    let (mut state,_)=plan_metadata_batch_owned_for_test(admission.staged_state().unwrap(),authority,&operations).unwrap();
    unica_metadata_designer::report_variants_storage::validate_final(&mut state.proposal(),"main",SourceSetKind::Configuration,operations.iter().map(|op|op.operation())).unwrap();
}
