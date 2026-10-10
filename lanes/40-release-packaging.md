# Lane 40 — release-packaging
Branch: `lane/40-release` | Status: QUEUED

## Ziel
Release + Paketierung (crates.io, static binary).

## Aufgaben
- `cargo build --release` + LTO-Profile, Version aus Git, Changelog, `INSTALL-RUST.md`.
- Kein opam/dune-Pflicht mehr für Lean-Pfad.

## Done
- `sail-rs --version` + `gen_lean` an `aarch64_small` aus released Binary.
