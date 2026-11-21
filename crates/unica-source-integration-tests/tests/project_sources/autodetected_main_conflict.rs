//! rule: INV.SOURCE.AUTODETECTED-MAIN-CONFLICT
use super::{
    discover_project_source_admission, discover_project_source_map,
    discover_project_source_map_controlled,
};
use std::fs;
use unica_source_contract::AutodetectedMainConflict;

#[test]
fn all_discovery_routes_retain_the_same_selected_and_skipped_main_roots() {
    for (roots, selected, skipped) in [
        (
            vec!["src/cf", "src/cfe/main", "src/extensions/main"],
            "src/cf",
            vec!["src/cfe/main", "src/extensions/main"],
        ),
        (
            vec!["src/cfe/main", "src/extensions/main"],
            "src/cfe/main",
            vec!["src/extensions/main"],
        ),
        (vec!["src/extensions/main"], "src/extensions/main", vec![]),
        (
            vec!["src/cf", "src/cfe", "src/cfe/main", "src/extensions/main"],
            "src/cf",
            vec!["src/extensions/main"],
        ),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        for relative in roots {
            let directory = root.join(relative);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        }
        let expected = (!skipped.is_empty()).then(|| AutodetectedMainConflict {
            selected_path: selected.into(),
            skipped_paths: skipped.iter().map(|path| (*path).into()).collect(),
        });
        let ordinary = discover_project_source_map(&root).unwrap();
        let controlled = discover_project_source_map_controlled(&root, &mut || Ok(())).unwrap();
        let admission = discover_project_source_admission(&root, &mut || Ok(())).unwrap();
        for map in [&ordinary, &controlled, admission.map()] {
            assert_eq!(map.autodetected_main_conflict, expected);
            let main = map
                .source_sets
                .iter()
                .filter(|source_set| source_set.name == "main")
                .collect::<Vec<_>>();
            assert_eq!(main.len(), 1);
            assert_eq!(main[0].path, selected);
            assert!(skipped.iter().all(|path| map
                .source_sets
                .iter()
                .all(|source_set| source_set.path != *path)));
        }
    }
}
