//! `sail-rs`-CLI — Lane-45-Treiber (E2E-Skizze Parse→Lean, E3: keine Builds).
#![forbid(unsafe_code)]

mod e2e;

fn usage() -> ! {
    eprintln!(
        "usage: sail-rs --e2e [input.sail] --out-dir <dir>\n\
         default input: sail/test/lean/SailTinyArm.sail (ab Repo-Root)"
    );
    std::process::exit(2);
}

fn main() {
    let mut args = e2e::E2eArgs::tiny_arm_default();
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|a| a == "--help" || a == "-h") {
        usage();
    }
    let mut i = 0usize;
    let mut positional: Option<String> = None;
    while i < raw.len() {
        match raw[i].as_str() {
            "--e2e" => {
                i += 1;
                if i < raw.len() && !raw[i].starts_with("--") {
                    positional = Some(raw[i].clone());
                    i += 1;
                }
            }
            "--out-dir" => {
                i += 1;
                if i >= raw.len() {
                    usage();
                }
                args.out_dir = raw[i].clone();
                i += 1;
            }
            s if s.starts_with("--") => {
                eprintln!("unbekanntes Flag {s} (Flag-Paritaet: Lane 38)");
                usage();
            }
            s => {
                positional = Some(s.to_string());
                i += 1;
            }
        }
    }
    if let Some(p) = positional {
        args.input = p;
    }

    let outcome = e2e::run(&args);
    for f in &outcome.emitted {
        println!("emitted {f}");
    }
    if outcome.fails.is_empty() {
        println!("e2e ok");
    } else {
        for f in &outcome.fails {
            eprintln!("{f}");
        }
        eprintln!(
            "naechster Schritt (Integrator, NICHT hier wegen E3): \
             lake build im out-dir pruefen"
        );
        std::process::exit(1);
    }
}
