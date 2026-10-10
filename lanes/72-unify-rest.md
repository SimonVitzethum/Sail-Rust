# Lane 72 — unify-rest (KindEnv/TypeEnv/Driver-Tabelle)
Branch: `lane/72-unify-rest` | Status: AKTIV

## Ziel
Lane-61-Reste mit Design-Entscheid: KindEnv/TypeEnv (Env.t fehlt in sail-ast —
entweder nach sail-ast holen oder begründet lokal lassen) + Driver-File-Tabelle
(Line/Col-Auflösung zentral).

## Aufgaben
- Entscheid je Punkt + Umsetzung, Tests als Code (NICHT ausführen, E3).
- `#![forbid(unsafe_code)]`.

## Done
- Alle Lane-61-Reste entschieden/umgesetzt, NEEDED zu.
- Entscheid KindEnv/TypeEnv (MIGRATION #23/#24): BEGRUENDET LOKAL in
  `sail-ty` (`src/ast_env.rs`: `kenv_to_kopts`/`kenv_from_kopts`,
  `TypeEnv::val_scheme_ast`/`register_scheme_ast`; `KindEnv::iter` neu).
  `sail-ast` bleibt reine Daten-Crate; `Env.t` ist Checker-Zustand.
- Entscheid Driver-File-Tabelle (MIGRATION #3/#25): ZENTRAL als
  `sail_ast::FileTable` (`src/files.rs`: `add`/`line_col`/`span_lines`/
  `line_text`/`snippet`, serde-stabil). `sail-diag` bleibt dependency-frei.
- Tests als Code (E3, nicht ausgefuehrt): `sail-ast/tests/file_table.rs`,
  `sail-ty/tests/env_ast_bridge.rs` (+ `ast_env`-Modultests),
  `sail-ast/src/files.rs`-Modultests.
