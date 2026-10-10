# Release-Ablauf (sail-rs, reines Rust)

Reiner Rust-Sail-Compiler, `#![forbid(unsafe_code)]`.
Kein opam/dune für den Lean-Pfad nötig (siehe `INSTALL-RUST.md`).

## 1. Vorbereitung

- [ ] `CHANGELOG.md` unter `Unreleased` befüllt, neues Tag-Segment angelegt.
- [ ] MSRV beachten: Rust >= 1.75.0.
- [ ] Sicherstellen, dass `git status` sauber ist
      (`git describe --dirty` muss OHNE `-dirty` ausgeben).

## 2. Release-Profil einbauen (Main-Maintainer)

Snippet aus `docs/release-profiles.toml.md` in die Workspace-`Cargo.toml`
des Main-Branches übernehmen (LTO-Fat, `codegen-units = 1`, `strip = true`).
Diese Lane liefert nur den Vorschlag, kein Cross-Branch-Edit.

## 3. Version verdrahten

`--version`-Flag gemäß `docs/CLI_NEEDED.md` an `sail-cli` andocken
(`build.rs` mit `git describe`, Fallback `CARGO_PKG_VERSION`).

## 4. Bauen + verifizieren (KEIN Full-ARM-Build, KEIN Publish)

```sh
cargo build --release -p sail-cli
./target/release/sail-rs --version          # muss "sail-rs <version>" ausgeben
./target/release/sail-rs gen_lean --model aarch64_small -o out/
cargo test --workspace
```

Done-Kriterium dieser Lane:

> `sail-rs --version` + `gen_lean` an `aarch64_small` aus released Binary.

## 5. Tag + Changelog

```sh
git tag -s vX.Y.Z -m "sail-rs vX.Y.Z"
git push --tags
```

Danach `CHANGELOG.md`-Eintrag von `Unreleased` nach `vX.Y.Z` verschieben.

## 6. Nicht tun

- KEIN `cargo publish` aus dieser Lane (crates.io bleibt späterem Schritt).
- KEIN Full-ARM-Build als Gate (nur `aarch64_small`-Smoke).
- `Cargo.lock` nie committen, `target/` nie committen (`.gitignore`).
