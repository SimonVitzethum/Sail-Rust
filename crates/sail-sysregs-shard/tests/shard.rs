//! Integrations-Tests ueber kleine Fixtures (kein Gesamt-Build, kein Lake).

#![forbid(unsafe_code)]

use sail_sysregs_shard::{run_shard, ShardConfig};
use std::fs::File;
use std::io::BufReader;

fn fixture(name: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    dir.join("tests")
        .join("fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn count_defs(path: &str) -> usize {
    let file = File::open(path).unwrap();
    sail_sysregs_shard::DefSplitter::new(BufReader::new(file)).count()
}

#[test]
fn pairs_fixture_has_six_defs() {
    assert_eq!(count_defs(&fixture("pairs.sail")), 6);
}

#[test]
fn tricky_fixture_stays_two_defs() {
    // Klammern in Kommentaren/Strings duerfen keine Defs aufspalten.
    assert_eq!(count_defs(&fixture("tricky.sail")), 2);
}

#[test]
fn runner_shards_pairs_into_blocks() {
    let cfg = ShardConfig { defs_per_block: 2 }.validated().unwrap();
    let report = run_shard(&fixture("pairs.sail"), cfg).unwrap();
    assert_eq!(report.total_defs, 6);
    assert_eq!(report.block_count(), 3);
    assert_eq!(report.total_lines, 24);
    assert!(report.peak_rss_kb.unwrap_or(0) > 0);
    // Fingerprints sind deterministisch.
    let again = run_shard(&fixture("pairs.sail"), cfg).unwrap();
    assert_eq!(report.combined_fingerprint(), again.combined_fingerprint());
    assert_eq!(report.blocks.len(), again.blocks.len());
    for (a, b) in report.blocks.iter().zip(again.blocks.iter()) {
        assert_eq!(a.fingerprint, b.fingerprint);
    }
}

#[test]
fn runner_handles_tricky_block() {
    let cfg = ShardConfig { defs_per_block: 64 }.validated().unwrap();
    let report = run_shard(&fixture("tricky.sail"), cfg).unwrap();
    assert_eq!(report.total_defs, 2);
    assert_eq!(report.block_count(), 1);
}
