//! Drop-in-Test: `gen_lean`-Aufruf aus `arm-v9.4-a/Makefile` laeuft mit
//! `sail-rs` (Dry-Run; kein Full-Build) + Exit-Code-Vertrag.
//!
//! Der argv-Block unten ist nahezu wortgleich mit:
//! ```make
//! $(SAIL) -dprofile $(VERBOSE_FLAG) -memo_z3 \
//!         -lean --lean-force-output \
//!         -lean_output_dir lean -o armv9 \
//!         -undefined_gen -memo-z3 -lean-noncomputable \
//!         -lean-line-width 100000 -lean-force-output \
//!         -lean-import-file lean/ArmExtras.lean \
//!         $(SAIL_SRCS) $(SAIL_FLAGS) src/termination_measures.sail
//! ```
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sail-rs"))
}

fn gen_lean_argv(extra: &[&str]) -> Vec<String> {
    let mut args = vec![
        "-dprofile",
        "-verbose",
        "1",
        "-memo_z3",
        "-lean",
        "--lean-force-output",
        "-lean_output_dir",
        "lean",
        "-o",
        "armv9",
        "-undefined_gen",
        "-memo-z3",
        "-lean-noncomputable",
        "-lean-line-width",
        "100000",
        "-lean-force-output",
        "-lean-import-file",
        "lean/ArmExtras.lean",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    args.extend(extra.iter().map(|s| s.to_string()));
    args.push("--dry-run".to_string());
    args
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sail-rs-lane38-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn gen_lean_dry_run_exits_zero() {
    let out = Command::new(binary())
        .args(gen_lean_argv(&[
            "src/prelude.sail",
            "src/termination_measures.sail",
        ]))
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("dry-run plan"), "{stdout}");
    assert!(stdout.contains("target=lean"), "{stdout}");
    assert!(stdout.contains("prefix=armv9"), "{stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[profile] phase=\"total\""),
        "dprofile fehlt: {stderr}"
    );
}

#[test]
fn gen_lean_dry_run_via_env_without_flag() {
    let out = Command::new(binary())
        .args(
            gen_lean_argv(&["a.sail"])
                .into_iter()
                .filter(|a| a != "--dry-run"),
        )
        .env("SAIL_RS_DRY_RUN", "1")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn help_lists_ocaml_compatible_flags() {
    let out = Command::new(binary()).arg("--help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    for needle in [
        "--lean",
        "--lean-output-dir",
        "--lean-force-output",
        "--lean-line-width",
        "--lean-extern-type",
        "-o <prefix>",
        "--memo-z3",
        "--dprofile",
        "usage: sail-rs <options>",
    ] {
        assert!(stdout.contains(needle), "fehlt: {needle}");
    }
}

#[test]
fn version_flag_exits_zero() {
    for flag in ["-v", "--version"] {
        let out = Command::new(binary()).arg(flag).output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&out.stdout).contains("sail-rs"));
    }
}

#[test]
fn unknown_flag_exits_1() {
    let out = Command::new(binary()).arg("--bogus-flag").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown option"));
}

#[test]
fn missing_input_file_exits_2() {
    let out = Command::new(binary())
        .args([
            "--just-check",
            "--no-core",
            "/tmp/sail-rs-lane38-does-not-exist.sail",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot open input file"));
}

#[test]
fn require_version_violation_exits_1() {
    let out = Command::new(binary())
        .args(["--require-version", "999.0.0", "x.sail", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("older than requested version"));
}

#[test]
fn ordinary_lean_run_rejects_missing_stages_without_stub_output() {
    let dir = temp_dir("incomplete");
    let input = dir.join("mini.sail");
    std::fs::write(&input, "val foo : unit -> int\nfunction foo() = 1\n").unwrap();
    let outdir = dir.join("lean");
    let out = Command::new(binary())
        .args([
            "--lean",
            "--no-core",
            "-dprofile",
            "--lean-output-dir",
            outdir.to_str().unwrap(),
            "--lean-force-output",
            "-o",
            "armv9",
            input.to_str().unwrap(),
        ])
        .env_remove("SAIL_RS_DRY_RUN")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("frontend converted 2 definitions"),
        "{stderr}"
    );
    assert!(stderr.contains("compilation incomplete"), "{stderr}");
    assert!(stderr.contains("phase=\"frontend\""), "{stderr}");
    assert!(
        !outdir.exists(),
        "incomplete compiler published an output project"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn just_check_rejects_invalid_sail_and_unresolved_includes() {
    for (label, source, message) in [
        ("syntax", "function bad() = `bad`\n", "unexpected character"),
        (
            "include",
            "$include <missing-library.sail>\n",
            "unavailable",
        ),
        ("condition", "$else\n", "$else without conditional"),
    ] {
        let dir = temp_dir(label);
        let input = dir.join("bad.sail");
        std::fs::write(&input, source).unwrap();
        let out = Command::new(binary())
            .args(["--just-check", "--no-core", input.to_str().unwrap()])
            .env_remove("SAIL_RS_DRY_RUN")
            .env_remove("SAIL_RS_LIBDIR")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{label}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(message), "{label}: {stderr}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn includes_macros_attributes_and_fixities_keep_order_across_workers() {
    let dir = temp_dir("ordered-includes");
    let input = dir.join("main.sail");
    std::fs::write(
        dir.join("ops.sail"),
        "$define ENABLE\ninfixl 6 %%\n$[never_fold] val f : unit -> int\n",
    )
    .unwrap();
    std::fs::write(&input,
        "$include \"ops.sail\"\n$ifdef ENABLE\nfunction f() = 1 %% 2\n$else\nfunction ignored() = 0\n$endif\n").unwrap();
    for workers in ["1", "4"] {
        let out = Command::new(binary())
            .args(["--just-check", "--no-core", input.to_str().unwrap()])
            .env_remove("SAIL_RS_DRY_RUN")
            .env("SAIL_RS_THREADS", workers)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("frontend converted 5 definitions from 2 files"),
            "{stderr}"
        );
        assert!(
            stderr.contains(&format!("using {workers} workers")),
            "{stderr}"
        );
        assert!(stderr.contains("compilation incomplete"), "{stderr}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn arm_legacy_trace_include_contains_real_declarations_and_bodies() {
    let dir = temp_dir("legacy-trace");
    let input = dir.join("main.sail");
    std::fs::write(&input, "$include <trace.sail>\n").unwrap();
    let out = Command::new(binary())
        .args(["--lean", "--no-core", input.to_str().unwrap()])
        .env_remove("SAIL_RS_DRY_RUN").env_remove("SAIL_RS_LIBDIR")
        .env("SAIL_RS_THREADS", "2").output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr.contains("frontend converted 8 definitions from 2 files"), "{stderr}");
    assert!(stderr.contains("compilation incomplete"), "{stderr}");
    std::fs::remove_dir_all(dir).unwrap();
}
