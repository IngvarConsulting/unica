use super::{write, RealReaderFixture, ViewRequest};
use unica_workspace::test_support::tree_snapshot;

fn install_handler(fixture: &RealReaderFixture) {
    let config = fixture.source.join("Configuration.xml");
    let text = std::fs::read_to_string(&config).unwrap();
    write(
        &config,
        &text.replace(
            "<ScheduledJob>MonthClose</ScheduledJob>",
            "<ScheduledJob>MonthClose</ScheduledJob><CommonModule>MonthClose</CommonModule>",
        ),
    );
    let descriptor =
        std::fs::read_to_string(fixture.source.join("CommonModules/РеактивныйСервер.xml")).unwrap();
    write(
        &fixture.source.join("CommonModules/MonthClose.xml"),
        &descriptor.replace("РеактивныйСервер", "MonthClose"),
    );
    write(
        &fixture
            .source
            .join("CommonModules/MonthClose/Ext/Module.bsl"),
        "Procedure RunScheduled() Export\nEndProcedure\n",
    );
}

#[test]
fn scheduled_job_handler_opens_the_observed_method_without_writes() {
    let fixture = RealReaderFixture::new();
    install_handler(&fixture);
    let before = tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let job = service.view(ViewRequest::new("main:ScheduledJob.MonthClose").unwrap());
    assert!(job.ok, "{job:?}");
    let data = job.data.unwrap();
    let at = data["props"]["implementationAt"]
        .as_str()
        .expect("the observed handler must have an openable address");
    assert_eq!(at, "main:CommonModule.MonthClose.Method.RunScheduled");
    let method = service.view(ViewRequest::new(at).unwrap());
    assert!(method.ok, "{method:?}");
    assert_eq!(method.data.unwrap()["kind"], "Method");
    assert_eq!(data["props"]["handlerMethod"], "RunScheduled");
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn scheduled_job_handler_never_invents_a_link_for_missing_or_damaged_targets() {
    for mutation in [
        "missing-owner",
        "missing-method",
        "invalid-bsl",
        "empty",
        "invalid-binding",
    ] {
        let fixture = RealReaderFixture::new();
        install_handler(&fixture);
        let descriptor = fixture.source.join("ScheduledJobs/MonthClose.xml");
        let module = fixture
            .source
            .join("CommonModules/MonthClose/Ext/Module.bsl");
        match mutation {
            "missing-owner" => {
                let config = fixture.source.join("Configuration.xml");
                let text = std::fs::read_to_string(&config).unwrap();
                write(
                    &config,
                    &text.replace("<CommonModule>MonthClose</CommonModule>", ""),
                );
            }
            "missing-method" => write(&module, "Procedure Other() Export\nEndProcedure\n"),
            "invalid-bsl" => write(&module, "Procedure RunScheduled(\n"),
            "empty" | "invalid-binding" => {
                let text = std::fs::read_to_string(&descriptor).unwrap();
                write(
                    &descriptor,
                    &text.replace(
                        "CommonModule.MonthClose.RunScheduled",
                        if mutation == "empty" {
                            ""
                        } else {
                            "CommonModule.MonthClose.Invalid.Name"
                        },
                    ),
                );
            }
            _ => unreachable!(),
        }
        let before = tree_snapshot(&fixture.source);
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:ScheduledJob.MonthClose").unwrap());
        assert!(result.ok, "{mutation}: {result:?}");
        let data = result.data.unwrap();
        assert!(
            data["props"]["implementationAt"].is_null(),
            "{mutation}: {data}"
        );
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}

#[test]
fn scheduled_job_handler_uses_declaration_case_and_rechecks_changed_source() {
    let fixture = RealReaderFixture::new();
    install_handler(&fixture);
    let module = fixture
        .source
        .join("CommonModules/MonthClose/Ext/Module.bsl");
    write(&module, "Procedure runscheduled() Export\nEndProcedure\n");
    // Each public invocation receives a fresh admitted reader; a reader's
    // projection cache belongs to its one retained operation.
    let view = || {
        fixture
            .view_service()
            .view(ViewRequest::new("main:ScheduledJob.MonthClose").unwrap())
    };
    assert_eq!(
        view().data.unwrap()["props"]["implementationAt"],
        "main:CommonModule.MonthClose.Method.runscheduled"
    );
    write(&module, "Procedure Different() Export\nEndProcedure\n");
    let result = view();
    assert!(result.ok);
    assert!(result.data.unwrap()["props"]["implementationAt"].is_null());
    write(&module, "Procedure RunScheduled() Export\nEndProcedure\nProcedure runscheduled() Export\nEndProcedure\n");
    assert!(view().data.unwrap()["props"]["implementationAt"].is_null());
}
