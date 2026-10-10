# Lane-15-Branch-Schutz: Regeln fuer Merges nach main
#
# Jede Lane arbeitet auf eigenem Branch `lane/<name>` im eigenen Worktree.
# Merges nach main erfolgen AUSSCHLIESSLICH per Pull Request.

## Pflicht-Checks (alle gruene Haken erforderlich)

| Check            | Was                              | Woher              |
|------------------|----------------------------------|--------------------|
| `fmt`            | `cargo fmt --all -- --check`     | `scripts/check.sh` |
| `clippy`         | `cargo clippy -- -D warnings`    | `scripts/check.sh` |
| `test`           | `cargo test --workspace`         | CI                 |
| `unsafe-verbot`  | `forbid(unsafe_code)` + kein `unsafe`-Token | `scripts/check.sh` |
| `geiger`         | `cargo geiger --forbid-only` (0 unsafe) | CI            |
| `msrv`           | Bau mit gepinnter MSRV-Toolchain | CI (`rust-version` aus Workspace-`Cargo.toml`) |

## Lane-spezifische Merge-Voraussetzungen

1. **Lane-13-Report (Pflicht):** Jeder PR, der Codegen/Printer/Rewrites
   beruehrt, braucht den differentiellen Oracle-Report aus Lane 13
   (OCaml-Sail vs. Rust-Output, `sail/test/lean/*.sail` + ARM-Subset).
   Ohne Report kein Merge — keine Ausnahmen.
2. **Lane-14-Budget-Check (Pflicht):** Jeder PR braucht gemessene Peak-RSS
   gegen `perf/BUDGETS.md` (`aarch64_small` <4GB, `arm-v8.5-a` <16GB,
   `arm-v9.4-a` <24GB). Bei Ueberschreitung: Shard verkleinern oder
   Pass in Lane 07/09 zurueckweisen, kein Merge.

## Weitere Regeln

- Kein direkter Push auf `main`; kein `--no-verify`-Merge.
- Pro Lane genau ein Worktree (`sail-rs/worktrees/lane-NN`); nach Merge:
  `git worktree remove sail-rs/worktrees/lane-NN` + Branch loeschen.
- `unsafe_code = "forbid"` steht zusaetzlich als Workspace-Lint in der
  Workspace-`Cargo.toml` — ein einzelnes Crate kann das Verbot nicht
  lokal aufweichen (`allow` greift bei `forbid` nicht).
- Ausnahme-Antraege fuer `unsafe` (z.B. `sail-solver`/Z3-FFI): nur per
  eigenem PR mit Begruendung + Lane-13/14-Nachweis; die Ausnahme gilt
  dann ausschliesslich fuer die fremde Dependency, nie fuer eigenen Code.
