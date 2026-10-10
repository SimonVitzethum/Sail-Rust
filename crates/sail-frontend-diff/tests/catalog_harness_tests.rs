#![forbid(unsafe_code)]
//! Catalog + harness-constructor tests (Lane 86, E3: code only, no runs).

use sail_frontend_diff::catalog::{CASES, Focus, MAX_FILE_BYTES, Tier, by_tier, find, tier_bytes};
use sail_frontend_diff::harness::{
    DUMP_INITIAL, DUMP_TC, FrontendConfig, FrontendStatus, build_dump_command,
};
use std::path::Path;

#[test]
fn catalog_has_36_sorted_cases() {
    assert_eq!(CASES.len(), 36);
    let mut stems: Vec<&str> = CASES.iter().map(|c| c.stem).collect();
    let mut sorted = stems.clone();
    sorted.sort();
    assert_eq!(stems, sorted, "CASES must stay sorted by stem");
    stems.dedup();
    assert_eq!(stems.len(), 36, "stems must be unique");
}

#[test]
fn catalog_sizes_current() {
    // Pinned Oct 2026; drift (upstream regen) must fail loudly.
    let get = |stem: &str| find(stem).expect("stem in catalog").bytes;
    assert_eq!(get("trivial"), 49);
    assert_eq!(get("let"), 582);
    assert_eq!(get("riscv_duopod"), 6698);
    assert_eq!(get("map_tactics"), 15724);
    assert_eq!(get("SailTinyArm"), 18171);
    assert_eq!(get("SailTinyArmV2"), 36899);
}

#[test]
fn catalog_tiers_partition_and_cap_consistent() {
    let smoke = by_tier(Tier::Smoke).len();
    let standard = by_tier(Tier::Standard).len();
    let nightly = by_tier(Tier::Nightly).len();
    assert_eq!(smoke + standard + nightly, CASES.len());
    assert_eq!(nightly, 3, "only the 3 over-cap files are nightly");
    for c in CASES {
        if c.bytes > MAX_FILE_BYTES {
            assert_eq!(c.tier, Tier::Nightly, "{}", c.stem);
        } else {
            assert_ne!(c.tier, Tier::Nightly, "{}", c.stem);
        }
    }
    assert_eq!(MAX_FILE_BYTES, 8192, "shared cap with sail-oracle");
    // Smoke tier is the fast gate: 8 tiny files.
    assert_eq!(smoke, 8);
    assert!(tier_bytes(Tier::Smoke) < 4096);
}

#[test]
fn catalog_errors_case_is_parse_only() {
    let errors = find("errors").expect("errors case");
    assert_eq!(errors.focus, Focus::ParseOnly);
    // Every other case wants both levels.
    for c in CASES.iter().filter(|c| c.stem != "errors") {
        assert_eq!(c.focus, Focus::ParseAndTy, "{}", c.stem);
    }
    assert!(find("no-such-fixture").is_none());
}

#[test]
fn dump_command_carries_flag_and_limits() {
    let cfg = FrontendConfig {
        timeout_secs: 45,
        memory_mb: 1,
        ..FrontendConfig::default()
    };
    for flag in [DUMP_INITIAL, DUMP_TC] {
        let (prog, args) = build_dump_command(&cfg, Path::new("trivial.sail"), flag);
        assert_eq!(prog, "bash");
        assert!(args[1].contains("ulimit -v 1024"), "script: {}", args[1]);
        assert!(args[1].contains("timeout 45s"), "script: {}", args[1]);
        assert!(args.contains(&"--just-check".to_string()));
        assert!(args.contains(&flag.to_string()));
        assert!(args.contains(&"trivial.sail".to_string()));
    }
}

#[test]
fn frontend_status_failure_set() {
    assert!(!FrontendStatus::ParseMatch.is_failure());
    assert!(!FrontendStatus::Skipped.is_failure());
    assert!(FrontendStatus::ParseMismatch.is_failure());
    assert!(FrontendStatus::TySampleMismatch.is_failure());
    assert!(FrontendStatus::OracleFailed.is_failure());
    assert!(FrontendStatus::Timeout.is_failure());
    assert!(FrontendStatus::OomSuspected.is_failure());
    assert!(FrontendStatus::HarnessError.is_failure());
    assert_eq!(FrontendStatus::ParseMatch.reason(), "parse trees match");
}

#[test]
fn default_config_matches_oracle_conventions() {
    let cfg = FrontendConfig::default();
    assert_eq!(cfg.memory_kb(), 2048 * 1024);
    assert_eq!(cfg.max_file_bytes, MAX_FILE_BYTES);
    assert_eq!(cfg.ty_samples_per_file, 8);
    assert!(cfg.sail_binary.to_string_lossy().ends_with("/sail"));
}
