# Sail-rs Release-Profil (Vorschlag, kein Eingriff in den Main-Workspace)

Der Workspace-`Cargo.toml` gehört dem Main-Branch. Diese Datei ist ein
Copy-paste-Snippet für den Maintainer — bewusst KEIN direktes Editieren
einer fremden `Cargo.toml` aus dieser Lane heraus.

## Einbau

Den folgenden Block ans Ende der Workspace-`Cargo.toml`
(`/home/simon/Dokumente/Sail/sail-rs/Cargo.toml`, im Main-Branch) anhängen:

```toml
[profile.release]
# Fette LTO für das ausgelieferte `sail-rs`-Binary (Lean-Pfad).
# Baut langsamer, Binary wird kleiner/schneller.
lto = "fat"
codegen-units = 1
strip = true
panic = "unwind"
```

Optionale schlanke Variante für CI-Smoke (schneller, ohne LTO):

```toml
[profile.release-smoke]
inherits = "release"
lto = false
codegen-units = 16
```

## Hinweise

- `panic = "unwind"` bleibt gesetzt, damit `cargo test --release`
  nicht an `panic=abort`-Tests scheitert.
- `strip = true` entfernt Debugsymbole aus dem Release-Binary.
  Für Crash-Analyse ggf. `strip = "debuginfo"` wählen.
- LTO greift workspace-weit erst ab Resolver v2
  (`resolver = "2"` ist im Workspace gesetzt).
- Reiner Rust-Build: `#![forbid(unsafe_code)]` bzw.
  `[workspace.lints.rust] unsafe_code = "forbid"` bleibt unverändert;
  dieses Profil ändert nur Codegen, keine Lints.
- Verifikation: `cargo build --release -p sail-cli` muss ohne
  opam/dune erfolgreich sein (siehe `INSTALL-RUST.md`).
