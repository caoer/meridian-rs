//! GT-pack smoke: the frozen pack is present, loadable, and internally
//! consistent (every expected.json parses and its `file` source exists).

use std::fs;

#[test]
fn gt_pack_loads() {
    let gt = testsuite::gt_pack_dir();
    let ground_truth = gt.join("ground-truth");

    let mut expected_files = 0;
    for entry in fs::read_dir(&ground_truth).expect("ground-truth dir exists") {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        expected_files += 1;

        let parsed: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{} is valid JSON: {e}", path.display()));

        let source_rel = parsed["file"]
            .as_str()
            .unwrap_or_else(|| panic!("{}: `file` field present", path.display()));
        let source = gt.join(source_rel);
        assert!(source.is_file(), "GT source {source_rel} present in pack");
        assert!(
            parsed["nodes"].is_array(),
            "{}: `nodes` is an array",
            path.display()
        );
    }

    assert_eq!(expected_files, 10, "the frozen pack carries 10 GT files");
}
