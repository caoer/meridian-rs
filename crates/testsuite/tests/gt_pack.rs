//! Smoke gate: the frozen GT pack loads — every expectation parses against
//! wire vocabulary, every source document resolves and is valid UTF-8, every
//! span is in bounds. The full parse-vs-GT harness arrives with the fork
//! (step 0); this test keeps the pack honest until then.

#[test]
fn gt_pack_loads() {
    let pack = testsuite::load_gt_pack();
    assert_eq!(pack.len(), 10, "the frozen pack carries exactly 10 expectation files");

    for (case, src) in &pack {
        assert!(!case.nodes.is_empty(), "{}: expectation has nodes", case.file);
        for node in &case.nodes {
            let wire::Span(start, end) = node.span;
            assert!(start <= end, "{}: span start ≤ end", case.file);
            assert!(
                end as usize <= src.len(),
                "{}: span [{start},{end}) inside {} bytes",
                case.file,
                src.len()
            );
            assert!(
                !node.text_prefix_16b.is_empty() || start == end,
                "{}: non-empty prefix window",
                case.file
            );
        }
    }
}

#[test]
fn gt_pack_is_the_frozen_one() {
    let frozen = std::fs::read_to_string(testsuite::gt_root().join("FROZEN")).expect("FROZEN header present");
    assert!(frozen.contains("frozen_at: 2026-07-17T23:48:52Z"), "provenance header intact");
    assert!(frozen.contains("ground_truth_files: 10"), "pack size matches provenance");
}
