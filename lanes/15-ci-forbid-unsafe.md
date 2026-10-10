# Lane 15 — ci-forbid-unsafe
Branch: `lane/15-ci` | Worktree: `sail-rs/worktrees/lane-15` (nach Merge löschen)

## Ziel
Reines Rust erzwingen + Workspace-Setup.

## Aufgaben
- Workspace-`Cargo.toml`, `#![forbid(unsafe_code)]` in jedem Crate, `cargo geiger` + `-D unsafe-code` in CI, `cargo fmt/clippy -D warnings`, MSRV pinnen.
- Branch-Schutz: Lane-Branches nur via PR mit Lane-13-Report + Lane-14-Budget-Check mergen.

## Done
- `cargo deny/geiger`-sauber, CI grün auf allen 15 Crates.
