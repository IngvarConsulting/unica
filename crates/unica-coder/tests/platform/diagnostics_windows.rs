#[cfg(windows)]
#[test]
fn diagnostics_windows_accepts_drive_letter_case_and_rejects_another_drive() {
    let fixture = Fixture::platform_xml();
    let module = fixture.write_common_module("Shared");
    let cancellation = CancellationToken::new();
    let context =
        resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();
    let mut swapped = module.to_string_lossy().into_owned();
    swapped.replace_range(0..1, &swapped[0..1].to_ascii_lowercase());
    assert!(map_diagnostic_observation(
        diagnostic(
            DiagnosticObservationLocation::Resource { handle: swapped },
            DiagnosticObservationFocus::Target,
        ),
        &context,
        &cancellation,
    )
    .is_ok());

    let current_drive = module.to_string_lossy().chars().next().unwrap();
    let other_drive = if current_drive.eq_ignore_ascii_case(&'Z') {
        'Y'
    } else {
        'Z'
    };
    let other = format!("{other_drive}:\\outside\\secret.bsl");
    let error = map_diagnostic_observation(
        diagnostic(
            DiagnosticObservationLocation::Resource { handle: other },
            DiagnosticObservationFocus::Target,
        ),
        &context,
        &cancellation,
    )
    .unwrap_err();
    assert_eq!(error.code, "location_outside_source_set");
    assert!(!error.message.contains("secret.bsl"));
}
