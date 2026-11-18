use super::*;

pub(super) fn install_storage_sources(fixture: &RealReaderFixture) {
    let config = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&config).unwrap();
    write(
        &config,
        &xml.replace(
            "</ChildObjects>",
            "<SettingsStorage>Probe</SettingsStorage></ChildObjects>",
        ),
    );
    for relative in [
        "SettingsStorages/Probe.xml",
        "SettingsStorages/Probe/Ext/ManagerModule.bsl",
        "SettingsStorages/Probe/Forms/Save.xml",
        "SettingsStorages/Probe/Forms/Save/Ext/Form.xml",
        "SettingsStorages/Probe/Forms/Save/Ext/Form/Module.bsl",
        "SettingsStorages/Probe/Forms/Empty.xml",
        "SettingsStorages/Probe/Forms/Empty/Ext/Form.xml",
    ] {
        write(
            &fixture.source.join(relative),
            &fixture_text(&format!(
                "acceptance/workspace-storage-form-module/src/{relative}"
            )),
        );
    }
}

/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
#[test]
fn storage_forms_publish_only_form_modules_and_keep_registration_boundaries() {
    let fixture = RealReaderFixture::new();
    install_storage_sources(&fixture);
    // A plausible file and descriptor do not register another owner.
    let descriptor = fs::read_to_string(fixture.source.join("SettingsStorages/Probe.xml")).unwrap();
    write(
        &fixture.source.join("SettingsStorages/Ghost.xml"),
        &descriptor.replace("Probe", "Ghost"),
    );
    write(
        &fixture
            .source
            .join("SettingsStorages/Ghost/Forms/Save/Ext/Form/Module.bsl"),
        "Процедура Ghost()\nКонецПроцедуры\n",
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    for form in ["Save", "Empty"] {
        let at = format!("main:SettingsStorage.Probe.Form.{form}.Module");
        let modules = service.view(ViewRequest::new(&at).unwrap());
        assert!(modules.ok, "{at}: {:?}", refusal_codes(&modules));
        let data = modules.data.unwrap();
        assert_eq!(data["items"].as_array().unwrap().len(), 1);
        assert_eq!(data["items"][0]["at"], format!("{at}.Form"));
    }
    for at in [
        "main:SettingsStorage.Probe.Form.Save.Module.Form.Method.ПриСозданииНаСервере",
        "main:SettingsStorage.Probe.Form.Save.Module.Form.Method.ПриОткрытии",
        "main:SettingsStorage.Probe.Module.Manager.Method.ManagerProbe",
    ] {
        let read = service.view(ViewRequest::new(at).unwrap());
        assert!(read.ok, "{at}: {:?}", refusal_codes(&read));
        assert_eq!(read.data.unwrap()["at"], at);
    }
    for at in [
        "main:SettingsStorage.Ghost.Form.Save.Module.Form",
        "main:SettingsStorage.Probe.Form.Missing.Module.Form",
        "main:SettingsStorage.Probe.Form.Save.Module.Command",
        "main:SettingsStorage.Probe.Form.Save.Module.Object",
        "main:SettingsStorage.Probe.Command.Open.Module.Command",
    ] {
        let read = service.view(ViewRequest::new(at).unwrap());
        assert_eq!(refusal_codes(&read), ["not_found"], "{at}: {read:?}");
    }
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
