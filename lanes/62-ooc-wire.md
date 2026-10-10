# Lane 62 — ooc-wire (OoC-Priorität)
Branch: `lane/62-ooc-wire` | Status: AKTIV

## Ziel
sail-ooc (Lane 60, Branch lane/60-ooc lesbar im Worktree lane-60) in den Pipeline-Treiber
verdrahten: Barrier um Rewrite-Pässe, Warm-Layer für fertige Shards, Memo-Spills für Solver.

## Aufgaben
- Treiber-Skizze in sail-cli (e2e.rs lesen): OoC-Hooks einbauen, ohne fremde Crates zu ändern.
- sail-ooc aus lane-60-Worktree lesen (nicht mergen, nur nutzen).
- E3: KEINE Builds/Tests. `#![forbid(unsafe_code)]`.

## Done
- Verdrahtungs-Code + Doku, Verifikation via Test-Rig (später).

## Stand (Lane-62-Agent)
- `crates/sail-cli/src/ooc.rs` (neu): `OocDriver` (Ein-Shard-Barriere),
  `OocHooks` (warm/memo/summary), `NoopHooks`, `RecordingHooks`, `OocStats`;
  nur `sail-cli` geändert, keine fremden Crates.
- `crates/sail-cli/src/e2e.rs`: `fun_names`-Zensus, `stream_rewrite`
  (Barrier → Seal → Warm-Spill → Summary), `run_with_ooc`-Eintritt,
  `Outcome.ooc`-Zähler; `run()` bleibt ohne FS-Effekte (NoopHooks).
- Herkunft: `sail-ooc` (Lane 60) gelesen im Haupt-Repo
  (`/home/simon/Dokumente/Sail/sail-rs/crates/sail-ooc` — lane-60-Worktree
  fehlt, Haupt-Repo-Stand genutzt, nichts kopiert/gedoppelt). Echte
  Content-Keys (`ContentId`) + echte Rewrite-Pässe folgen mit
  Unified-AST (lane-47) bzw. `sail-ooc`-Bindung (`impl OocHooks`).
- E3: KEIN Build/Test ausgeführt (nur Code + Review); Tests als
  `#[cfg(test)]`-Code in `ooc.rs`/`e2e.rs`. `#![forbid(unsafe_code)]`
  (Crate-Root). `.gitignore` enthält `target/` + `Cargo.lock`.
