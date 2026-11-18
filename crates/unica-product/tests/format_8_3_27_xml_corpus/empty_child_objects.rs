use super::*;

fn assert_native_empty_collection(case_id: &str, owner_path: &str) {
    let root = tempfile::tempdir().unwrap();
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == case_id)
        .unwrap();
    let mut gate = SequentialCallGate::default();
    run_corpus_case(root.path(), case, &mut gate).unwrap();
    assert_eq!(gate.completed_target_calls, 1);
    let published = root
        .path()
        .join("cases")
        .join(case_id)
        .join("workspace")
        .join(owner_path);
    let source = fs::read_to_string(published).unwrap();
    let source = source.strip_prefix('\u{feff}').unwrap_or(&source);
    let document = Document::parse(source).unwrap();
    let owner = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .unwrap();
    let collection = owner
        .children()
        .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "ChildObjects")))
        .unwrap();
    // Both retained 8.3.27.2074 native exports agree on these exact bytes.
    assert_eq!(&source[collection.range()], "<ChildObjects/>", "{case_id}");
}

#[test]
fn form_removal_publishes_the_native_self_closing_owner_collection() {
    assert_native_empty_collection("form-remove-managed", "src/Catalogs/CorpusCatalog.xml");
}

#[test]
fn template_removal_publishes_the_native_self_closing_owner_collection() {
    assert_native_empty_collection(
        "template-remove-object-template",
        "src/Reports/CorpusReport.xml",
    );
}
