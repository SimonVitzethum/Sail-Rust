# Changelog

Alle nennenswerten Änderungen an Release/Verpackung dieser Lane.

## Unreleased

- `docs/RELEASE.md`: Release-Ablauf ohne opam/dune, Done = `--version` +
  `gen_lean` an `aarch64_small` aus Release-Binary.
- `docs/release-profiles.toml.md`: LTO-Release-Profil als Copy-paste-Vorschlag
  für die Main-Workspace-`Cargo.toml` (kein Cross-Branch-Edit).
- `INSTALL-RUST.md`: Rust-only-Buildanleitung für den Lean-Pfad.
- `docs/CLI_NEEDED.md`: Versionsstrategie `git describe` → `--version`-Flag
  als Andock-Spezifikation für `sail-cli`.
- `.gitignore`: `target/` und `Cargo.lock` werden nie committet.
