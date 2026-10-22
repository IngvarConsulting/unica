use super::{write, RealReaderFixture, ViewRequest};
use serde_json::{json, Value};
use unica_workspace::test_support::tree_snapshot;

const SCHEDULE: &str = include_str!("../../../../../../tests/fixtures/acceptance/workspace-job-schedule/src/ScheduledJobs/MonthClose/Ext/Schedule.xml");

fn read(fixture: &RealReaderFixture, at: &str) -> Value {
    let result = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert!(result.ok, "{at}: {result:?}");
    result.data.unwrap()
}

#[test]
fn job_schedule_is_discoverable_and_preserves_all_observed_facts_without_writes() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("ScheduledJobs/MonthClose/Ext/Schedule.xml"),
        SCHEDULE,
    );
    let before = tree_snapshot(&fixture.source);
    let owner = read(&fixture, "main:ScheduledJob.MonthClose");
    let branch = owner["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|branch| branch["at"] == "main:ScheduledJob.MonthClose.Schedule")
        .expect("the owner must advertise its schedule");
    let schedule = read(&fixture, branch["at"].as_str().unwrap());
    assert_eq!(
        schedule["props"],
        json!({
            "state":"present", "BeginDate":"0001-01-01", "EndDate":"0001-01-01",
            "BeginTime":"00:00:00", "EndTime":"00:00:00", "CompletionTime":"00:00:00",
            "CompletionInterval":0, "RepeatPeriodInDay":3600, "RepeatPause":3600,
            "WeekDayInMonth":0, "DayInMonth":0, "WeeksPeriod":1, "DaysRepeatPeriod":1,
            "WeekDays":"1 2 3 4 5 6 7", "Months":"1 2 3 4 5 6 7 8 9 10 11 12"
        })
    );
    assert_eq!(owner["props"]["Use"], true);
    assert_eq!(owner["props"]["Predefined"], false);
    assert_eq!(owner["props"]["handlerModule"], "CommonModule.MonthClose");
    assert_eq!(owner["props"]["handlerMethod"], "RunScheduled");
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn job_schedule_sidecar_changes_are_observed_on_the_same_service() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let at = "main:ScheduledJob.MonthClose.Schedule";
    let view = || {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(result.ok, "{result:?}");
        result.data.unwrap()["props"].clone()
    };
    assert_eq!(view(), json!({"state":"absent"}));
    let path = fixture
        .source
        .join("ScheduledJobs/MonthClose/Ext/Schedule.xml");
    write(&path, SCHEDULE);
    assert_eq!(view()["RepeatPeriodInDay"], 3600);
    write(&path, &SCHEDULE.replace("3600", "0900"));
    assert_eq!(view()["RepeatPeriodInDay"], 900);
    std::fs::remove_file(path).unwrap();
    assert_eq!(view(), json!({"state":"absent"}));
}

#[test]
fn job_schedule_preserves_complete_lists_split_by_xml_comments() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("ScheduledJobs/MonthClose/Ext/Schedule.xml"),
        &SCHEDULE.replace("1 2", "1<!--split--> 2"),
    );
    let props = read(&fixture, "main:ScheduledJob.MonthClose.Schedule")["props"].clone();
    assert_eq!(props["WeekDays"], "1 2 3 4 5 6 7");
    assert_eq!(props["Months"], "1 2 3 4 5 6 7 8 9 10 11 12");
}

#[test]
fn job_schedule_reads_platform_empty_lists_and_days_from_month_end() {
    // Платформа пишет пустые `<ent:WeekDays/>` и `<ent:Months/>` у расписания
    // без фильтра, а отрицательный день отсчитывает от конца месяца.
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("ScheduledJobs/MonthClose/Ext/Schedule.xml"),
        &SCHEDULE
            .replace(
                "<ent:WeekDays>1 2 3 4 5 6 7</ent:WeekDays>",
                "<ent:WeekDays/>",
            )
            .replace(
                "<ent:Months>1 2 3 4 5 6 7 8 9 10 11 12</ent:Months>",
                "<ent:Months/>",
            )
            .replace(
                "DayInMonth=\"0\" WeeksPeriod",
                "DayInMonth=\"-1\" WeeksPeriod",
            )
            .replace("WeekDayInMonth=\"0\"", "WeekDayInMonth=\"-2\""),
    );
    let props = &read(&fixture, "main:ScheduledJob.MonthClose.Schedule")["props"];
    assert_eq!(props["state"], "present");
    assert_eq!(props["WeekDays"], "");
    assert_eq!(props["Months"], "");
    assert_eq!(props["DayInMonth"], -1);
    assert_eq!(props["WeekDayInMonth"], -2);
}

#[test]
fn job_schedule_present_damage_never_becomes_absence_or_erases_declarations() {
    for xml in [
        "<broken",
        "<JobSchedule/>",
        &SCHEDULE.replace("version=\"2.20\"", "version=\"99\""),
        &SCHEDULE.replace("RepeatPause=\"3600\"", "RepeatPause=\"no\""),
        &SCHEDULE.replace("http://v8.1c.ru/8.3/xcf/extrnprops", "urn:foreign"),
        &SCHEDULE.replace("</Schedule>", "<Unknown/></Schedule>"),
        &SCHEDULE.replace("<ent:Months>", "<ent:Months foreign=\"x\">"),
        &SCHEDULE.replace("</Schedule>", "<ent:Months>1</ent:Months></Schedule>"),
        &SCHEDULE.replace("1 2 3 4 5 6 7</ent:WeekDays>", "1 8</ent:WeekDays>"),
        &SCHEDULE.replace("1 2 3 4 5 6 7</ent:WeekDays>", "1 1</ent:WeekDays>"),
        &SCHEDULE.replace("BeginDate=\"0001-01-01\"", "BeginDate=\"2026-02-30\""),
        &SCHEDULE.replace("BeginTime=\"00:00:00\"", "BeginTime=\"25:00:00\""),
        &SCHEDULE.replace("RepeatPause=\"3600\"", "RepeatPause=\"3600\" Future=\"1\""),
        &SCHEDULE.replace("version=\"2.20\"", "version=\"2.20\" Future=\"1\""),
    ] {
        let fixture = RealReaderFixture::new();
        write(
            &fixture
                .source
                .join("ScheduledJobs/MonthClose/Ext/Schedule.xml"),
            xml,
        );
        let owner = read(&fixture, "main:ScheduledJob.MonthClose");
        assert_eq!(owner["props"]["handlerMethod"], "RunScheduled");
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:ScheduledJob.MonthClose.Schedule").unwrap());
        assert!(
            !result.ok,
            "present unsupported or damaged XML must refuse: {xml}"
        );
        assert_eq!(super::refusal_codes(&result), vec!["provider_unavailable"]);
    }
}

#[test]
fn job_schedule_does_not_read_foreign_owners_or_cancelled_sources() {
    let fixture = RealReaderFixture::new();
    for at in [
        "main:Catalog.Items.Schedule",
        "main:ScheduledJob.Orphan.Schedule",
        "main:ScheduledJob.MonthClose.Schedule.Item",
    ] {
        let result = fixture.view_service().view(ViewRequest::new(at).unwrap());
        assert_eq!(super::refusal_codes(&result), vec!["not_found"], "{at}");
    }
    fixture.cancellation.cancel();
    let result = fixture
        .view_service()
        .view(ViewRequest::new("main:ScheduledJob.MonthClose.Schedule").unwrap());
    assert_eq!(super::refusal_codes(&result), vec!["cancelled"]);
}

#[test]
fn job_schedule_refuses_linked_artifacts_and_bounds_input() {
    use unica_workspace::test_support::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    let fixture = RealReaderFixture::new();
    let path = fixture
        .source
        .join("ScheduledJobs/MonthClose/Ext/Schedule.xml");
    write(&path, &"x".repeat(256 * 1024 + 1));
    let at = "main:ScheduledJob.MonthClose.Schedule";
    let result = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert_eq!(super::refusal_codes(&result), vec!["provider_unavailable"]);
    let foreign = fixture.root.path().join("foreign.xml");
    write(&foreign, SCHEDULE);
    std::fs::remove_file(&path).unwrap();
    let outcome = create_file_link_fixture_for_test(&foreign, &path).unwrap();
    if outcome != FileLinkFixtureOutcome::Created {
        eprintln!("[SKIPPED FIXTURE] file-link fixture unavailable: {outcome:?}");
        return;
    }
    let before = std::fs::read(&foreign).unwrap();
    let result = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert_eq!(super::refusal_codes(&result), vec!["provider_unavailable"]);
    assert_eq!(std::fs::read(foreign).unwrap(), before);
}

#[test]
fn job_schedule_checkpoints_preserve_cancellation_and_deadline() {
    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use unica_execution_contract::view_error::ViewError;
    use unica_refusal::RefusalCode;
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("ScheduledJobs/MonthClose/Ext/Schedule.xml"),
        SCHEDULE,
    );
    let authority = fixture.read_authority();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "ScheduledJob.MonthClose").unwrap();
    let mut total = 0;
    authority
        .read
        .job_schedule(&target, &mut || {
            total += 1;
            Ok(())
        })
        .unwrap();
    assert!(total >= 3);
    for code in [RefusalCode::Cancelled, RefusalCode::DeadlineExceeded] {
        for stop_at in [1, 2, total] {
            let mut count = 0;
            let error = authority
                .read
                .job_schedule(&target, &mut || {
                    count += 1;
                    if count == stop_at {
                        Err(ViewError::new(code, "schedule read interrupted"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.code(), code);
            assert_eq!(count, stop_at);
        }
    }
}

#[test]
fn job_schedule_resolves_to_its_file_only_while_the_file_exists() {
    use unica_address::address::QualifiedAddress;
    use unica_refusal::RefusalCode;
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    let owner = QualifiedAddress::parse("main:ScheduledJob.MonthClose").unwrap();
    let at = QualifiedAddress::parse("main:ScheduledJob.MonthClose.Schedule").unwrap();
    let file = "ScheduledJobs/MonthClose/Ext/Schedule.xml";
    // The absent schedule stays a successful view, but no file is located.
    assert_eq!(
        read(&fixture, "main:ScheduledJob.MonthClose.Schedule")["props"],
        json!({"state":"absent"})
    );
    for requested in [None, Some(file)] {
        let error = authority.existing_export_path(&at, requested).unwrap_err();
        assert_eq!(error.code(), RefusalCode::NotFound, "{requested:?}");
    }
    assert_eq!(
        authority.existing_export_path(&owner, None).unwrap(),
        "ScheduledJobs/MonthClose.xml"
    );
    write(&fixture.source.join(file), SCHEDULE);
    assert_eq!(authority.existing_export_path(&at, None).unwrap(), file);
    assert_eq!(
        authority
            .existing_export_path(&at, Some("ScheduledJobs/MonthClose.xml"))
            .unwrap_err()
            .code(),
        RefusalCode::NotFound
    );
    std::fs::remove_file(fixture.source.join(file)).unwrap();
    assert_eq!(
        authority
            .existing_export_path(&at, None)
            .unwrap_err()
            .code(),
        RefusalCode::NotFound
    );
}
