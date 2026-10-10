# Lane 111 — kindenv-final (Env.t-Entscheid)
Branch: `lane/111-kindenv-final` | Status: AKTIV

## Ziel
Lane-61/72-Rest endgültig: KindEnv/TypeEnv mit val-specs/registers/scattered
vollständig (Env.t-Umfang) ODER begründet eingefroren mit Doku wo Schluss ist.

## Aufgaben
- Umsetzen + ENV_COVERAGE.md, Tests als Code (NICHT ausführen, E3).
  `#![forbid(unsafe_code)]`.

## Done
- Env-Abdeckung vollständig oder explizit eingefroren.
- Lane 111 (E3, Tests als Code, nicht ausgefuehrt): `KindEnv`
  (`len`/`is_empty`/`remove` neu) + `TypeEnv` (val/register:
  `contains_*`/`iter_*`/*`_names`/*`_count`/`remove_*`, `local_count`/
  `clear_locals` neu) VOLLSTÄNDIG; Scattered EINGEFROREN
  (resolver-owned in `sail-resolve`, Checker sieht nur geschlossene
  Schemes via `add_val`). Freeze-Protokoll: `ENV_COVERAGE.md` (Root).
  Tests: `sail-ty/tests/env_coverage.rs` (neu) + Modultests
  (`env.rs`/`kind.rs`), `env_ast_bridge`/`env_serde_roundtrip` bestehend.
  `#![forbid(unsafe_code)]` ueberall.
