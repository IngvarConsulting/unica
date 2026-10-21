use super::test_support::*;
use super::*;
pub fn reference_scan_refuses_a_file_larger_than_the_per_file_budget(budget: usize) {
    let root = temp_root("reference-file-budget");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::File::create(root.join("Huge.xml"))
        .unwrap()
        .set_len(budget as u64 + 1)
        .unwrap();
    let mut state = staged(&root);
    let error = state
        .read_guarded_bounded(Path::new("Huge.xml"), budget)
        .unwrap_err();
    assert!(error.to_string().contains("bound"), "{error}");
    assert!(
        state.entries.is_empty(),
        "oversized input must not enter the staged byte set"
    );
    std::fs::write(root.join("Small.xml"), "\u{feff}<Root/>".as_bytes()).unwrap();
    assert_eq!(
        state
            .read_guarded_bounded(Path::new("Small.xml"), budget)
            .unwrap(),
        Some("\u{feff}<Root/>".as_bytes().to_vec())
    );
    assert!(state.entries.is_empty());
    assert_eq!(state.read_guards.len(), 1);
}
