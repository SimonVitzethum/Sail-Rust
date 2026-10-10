# INSTALL-RUST.md — sail-rs bauen ohne opam/dune (Lean-Pfad)

Für den Lean-Pfad (`sail-rs ... gen_lean ...`) werden **weder opam noch dune**
benötigt. Der reine Rust-Compiler ersetzt den OCaml-Build vollständig.

## Voraussetzungen

- Rust-Toolchain **>= 1.75.0** (MSRV, siehe Workspace-`Cargo.toml`).
  ```sh
  rustc --version   # muss >= 1.75.0 ausgeben
  ```
- Optional für Lean-Ausgabe prüfen: `lean --version`, `lake --version`
  (nur zum Typchecken des generierten Lean-Codes nötig, nicht zum Bauen).
- KEIN opam, KEIN dune, KEIN OCaml nötig.

## Bauen

```sh
# aus dem Workspace-Root (Main-Branch):
cargo build --release -p sail-cli
./target/release/sail-rs --version
```

Erwartung: `sail-rs --version` gibt die Git-Version aus
(siehe `docs/CLI_NEEDED.md` für die Verdrahtung).

## Lean generieren (Smoke, kein Full-ARM-Build)

```sh
./target/release/sail-rs gen_lean --model aarch64_small -o out/
```

Diese Lane verlangt bewusst **KEINEN Full-ARM-Build** und **KEIN Publish**:
Der Smoke gegen `aarch64_small` aus dem Release-Binary genügt als Done-Nachweis.

## Statisches Binary (optional)

Für ein portables Release-Binary:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl -p sail-cli
```

Hinweis: musl-Builds brauchen ggf. `musl-tools` im System.
Fällt das in der Umgebung aus, bleibt das normale
`cargo build --release` der Referenz-Release-Pfad.

## Tests

```sh
cargo test --workspace
```

`Cargo.lock` wird nie committet (`.gitignore`), `target/` ebenfalls nicht.
