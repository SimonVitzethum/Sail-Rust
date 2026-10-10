//! Tests: GNU time -v-Parser, Timings-Log, Budgets.

use super::*;
use std::io::Write;

const SAMPLE_TIME_V: &str = "\
\tCommand being timed: \"true\"\n\
\tUser time (seconds): 0.00\n\
\tSystem time (seconds): 0.00\n\
\tPercent of CPU this job got: 100%\n\
\tElapsed (wall clock) time (h:mm:ss or m:ss): 0:00.00\n\
\tAverage shared text size (kbytes): 0\n\
\tAverage unshared data size (kbytes): 0\n\
\tAverage stack size (kbytes): 0\n\
\tAverage total size (kbytes): 0\n\
\tMaximum resident set size (kbytes): 2304\n\
\tAverage resident set size (kbytes): 0\n\
\tMajor (requiring I/O) page faults: 0\n\
\tMinor (reclaiming a frame) page faults: 112\n\
\tVoluntary context switches: 1\n\
\tInvoluntary context switches: 0\n\
\tSwaps from disk (now unused): 0\n\
\tFile system inputs: 0\n\
\tFile system outputs: 0\n\
\tSocket messages sent: 0\n\
\tSocket messages received: 0\n\
\tSignals delivered: 0\n\
\tPage size (bytes): 4096\n\
\tExit status: 0\n";

const SAMPLE_SLEEP: &str = "\
\tCommand being timed: \"sleep 0.1\"\n\
\tUser time (seconds): 0.00\n\
\tSystem time (seconds): 0.00\n\
\tElapsed (wall clock) time (h:mm:ss or m:ss): 0:00.10\n\
\tMaximum resident set size (kbytes): 2048\n\
\tMajor (requiring I/O) page faults: 1\n\
\tMinor (reclaiming a frame) page faults: 90\n\
\tSwaps from disk (now unused): 0\n\
\tExit status: 0\n";

#[test]
fn parses_minimal_true_sample() {
    let r = parse_time_v_output(SAMPLE_TIME_V).expect("must parse");
    assert_eq!(r.max_rss_kb, 2304);
    assert_eq!(r.wall_s, 0.0);
    assert_eq!(r.user_s, 0.0);
    assert_eq!(r.sys_s, 0.0);
    assert_eq!(r.minor_faults, 112);
    assert_eq!(r.major_faults, 0);
    assert_eq!(r.swaps, 0);
    assert_eq!(r.command, "true");
    assert_eq!(r.exit_status, Some(0));
}

#[test]
fn parses_wall_clock_with_minutes_and_hours() {
    assert_eq!(parse_elapsed("0:00.10"), Some(0.10));
    assert_eq!(parse_elapsed("1:02.50"), Some(62.50));
    assert_eq!(parse_elapsed("1:02:03"), Some(3723.0));
    assert_eq!(parse_elapsed("0:01.23"), Some(1.23));

    let r = parse_time_v_output(SAMPLE_SLEEP).expect("must parse");
    assert_eq!(r.max_rss_kb, 2048);
    assert!((r.wall_s - 0.10).abs() < 1e-9);
    assert_eq!(r.command, "sleep 0.1");
}

#[test]
fn missing_rss_is_error() {
    let out = "\tElapsed (wall clock) time (h:mm:ss or m:ss): 0:00.01\n";
    assert!(parse_time_v_output(out).is_err());
}

#[test]
fn missing_wall_is_error() {
    let out = "\tMaximum resident set size (kbytes): 1234\n";
    assert!(parse_time_v_output(out).is_err());
}

#[test]
fn garbage_is_error() {
    assert!(parse_time_v_output("").is_err());
    assert!(parse_time_v_output("hello\nworld\n").is_err());
}

#[test]
fn timings_log_roundtrip_jsonl() {
    let dir = std::env::temp_dir().join("sail-perf-test-log");
    let _ = std::fs::remove_file(&dir);
    let a = PhaseRecord::new("parse", 12, 3456).with_note("tiny.sail shard=1");
    let b = PhaseRecord::new("typecheck", 34, 7890);
    append_record(&dir, &a).unwrap();
    append_record(&dir, &b).unwrap();
    // Leere Zeile tolerieren.
    {
        let mut f = OpenOptions::new().append(true).open(&dir).unwrap();
        f.write_all(b"\n").unwrap();
    }
    let back = read_records(&dir).unwrap();
    assert_eq!(back, vec![a, b]);
    assert_eq!(peak_of(&back), 7890);
    let _ = std::fs::remove_file(&dir);
}

#[test]
fn from_report_converts_wall_to_ms() {
    let r = parse_time_v_output(SAMPLE_SLEEP).unwrap();
    let rec = PhaseRecord::from_report("fixture", &r, "host=test");
    assert_eq!(rec.wall_ms, 100);
    assert_eq!(rec.peak_rss_kb, 2048);
}

#[test]
fn budgets_have_spec_values() {
    assert_eq!(BUDGETS[0], Budget::new("aarch64_small", 4));
    assert_eq!(BUDGETS[1], Budget::new("arm-v8.5-a", 16));
    assert_eq!(BUDGETS[2], Budget::new("arm-v9.4-a", 24));
    assert_eq!(BUDGETS[0].limit_kib, 4 * KIB_PER_GIB);
    assert!(BUDGETS[0].fits(1024));
    assert!(!BUDGETS[0].fits(4 * KIB_PER_GIB + 1));
    assert!(check_budget(BUDGETS[0], 1024).is_ok());
    assert!(check_budget(BUDGETS[0], u64::MAX).is_err());
}
