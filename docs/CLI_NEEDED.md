# CLI_NEEDED.md — Versionsstrategie (`git describe` → `--version`)

Die Crate `sail-cli` (Main-Branch, `crates/sail-cli`) gehört nicht dieser
Lane. Diese Datei spezifiziert die Schnittstelle, die der Maintainer dort
andockt — kein Eingriff in fremde Branches aus dieser Lane heraus.

## Ziel

```
sail-rs --version
# -> "sail-rs 0.1.0+g12ab34c"  (Tag + git-describe-Suffix)
```

Version = höchstes semver-Tag via `git describe --tags --dirty --always`,
Fallback = `CARGO_PKG_VERSION` (reproduzierbar ohne `.git`, z. B. crates.io).

## Vorschlag: `crates/sail-cli/build.rs` (neu, reines Rust, kein unsafe)

```rust
// build.rs — kein unsafe, nur Env + Prozessaufruf zur Buildzeit.
fn main() {
    let fallback = env!("CARGO_PKG_VERSION");
    let version = std::process::Command::new("git")
        .args(["describe", "--tags", "--dirty", "--always"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.to_owned());
    println!("cargo:rustc-env=SAIL_RS_VERSION={version}");
    // Rebuild nur bei Bedarf anstoßen:
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");
}
```

## Vorschlag: `--version`-Verdrahtung in `sail-cli`

- Handgeschriebener Parser: Flag `--version`/`-V` gibt
  `concat!("sail-rs ", env!("SAIL_RS_VERSION"))` auf stdout aus und
  beendet mit Code 0, ohne weitere Args zu verlangen.
- Mit clap-derive: `#[command(version = env!("SAIL_RS_VERSION"))]`.
- Der String MUSS mit `sail-rs ` beginnen, damit der Lane-40-Done-Check
  (`sail-rs --version`) grep-fähig bleibt.
- `#![forbid(unsafe_code)]` gilt auch für `build.rs`-nahen Code:
  obiger Vorschlag enthält kein `unsafe`.

## Tests (im Main-Branch nachzuziehen, hier nur spezifiziert)

- `--version` exiting 0 und Ausgabeformat `sail-rs <non-empty>`.
- Build ohne `.git`-Verzeichnis fällt auf `CARGO_PKG_VERSION` zurück.

## Referenzen

- `docs/RELEASE.md` — Release-Ablauf, der dieses Flag verifiziert.
- `docs/release-profiles.toml.md` — LTO-Profil für das Release-Binary.
