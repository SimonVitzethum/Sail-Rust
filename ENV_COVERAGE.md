# ENV_COVERAGE.md — KindEnv/TypeEnv-Endzustand (Lane 111)

Lane-61/72-Rest endgültig (MIGRATION.md #23/#24): `KindEnv`/`TypeEnv`
bleiben **begründet lokal** in `sail-ty` (`sail-ast` ist reine Daten-Crate;
OCaml `Env.t` (`env.ml`) ist Checker-*Zustand*, kein AST-Knoten).
Diese Datei friert ein, was davon **vollständig** ist und wo **Schluss** ist.

E3: Tests nur als Code, nicht ausgeführt.

## Entscheid

| `Env.t`-Ausschnitt | Status | Lage |
|---|---|---|
| Kind-Bindungen (`K_int`/`K_bool`/`K_type` + lokale `K_nat`-Refinement) | **VOLLSTÄNDIG** | `sail-ty/src/kind.rs`: `KindEnv` (`bind`/`lookup`/`iter`/`len`/`is_empty`/`remove`) + `check_nexp_kind`/`check_constraint_kind`/`check_typ_kind` |
| `val` specs (quantifizierte Schemes) | **VOLLSTÄNDIG** | `sail-ty/src/env.rs`: `TypeEnv::add_val`/`lookup_val`/`contains_val`/`iter_vals`/`val_names`/`val_count`/`remove_val` + `check_val_def`/`check_scheme` (`src/check.rs`) + kanonische Sicht `val_scheme_ast` (`src/ast_env.rs` → `sail_ast::TypSchm`) |
| Registers (quantifizierte Schemes) | **VOLLSTÄNDIG** | `TypeEnv::add_register`/`lookup_register`/`contains_register`/`iter_registers`/`register_names`/`register_count`/`remove_register` + `check_register` + `register_scheme_ast` |
| Locals (`let`-Scopes) | **VOLLSTÄNDIG** | `TypeEnv::add_local`/`lookup_local`/`lookup`/`local_count`/`clear_locals`; val/register-Namensräume bleiben getrennt, Locals shadowen |
| Snapshots für Worker | **VOLLSTÄNDIG** | Programmtabellen `Arc`-geteilt (Lane 68), `Clone` billig, Mutation/Removal CoW (`Arc::make_mut`); `Send + Sync`; `serde`-stabil (Lane 65) |
| Scattered open/`impl`/`end`-Zustand | **EINGEFROREN (resolver-owned)** | Bleibt in `sail-resolve` (`env.rs`: `scattered_functions/_mappings/_variants/_enums`, `insert_scattered`/`lookup_scattered`; `resolve.rs`: open/impl/end-Verkettung, `DanglingImpl`/`DanglingScatteredClause`/`DanglingScatteredEnd`). Der Checker sieht nur *geschlossene* Schemes via `add_val`. Kein Duplikat-State in `sail-ty` — Begründung: offener Streu-Zustand ist Auflösungs-, nicht Prüflogik; zwei Quellen würden divergieren |
| `extern`-Bindings an `val` specs | **EINGEFROREN** | Gehört zu `sail_ast::ValSpec.extern_opt` (Daten) / Lane 25 (FFI-Mapping); kein Checker-State |
| Overloads, Fixities, Instantiations, Typ-Abbrevs | **EINGEFROREN (out of scope)** | Daten (`sail_ast::DefKind`) bzw. andere Lanes (30/62/83); kein `Env.t`-Rest dieser Lane |

## Vollständigkeits-Regeln (eingefroren)

- Re-Deklaration: **last wins**, keine Duplikat-Fehler auf Env-Ebene
  (Duplikate meldet der Resolver mit Spans; der Checker hält Schemes).
- `val`/`register` sind **getrennte Namensräume** (gleicher Name in beiden
  ist legal und unabhängig löschbar).
- `lookup` projiziert nur **monomorphe** Schemes (`quants.is_empty()`);
  polymorphe Nutzung läuft über `instantiate_scheme` + Solver
  (`infer`/`Var`-Fall, Regeln `Q-Inst`/`I-VarPoly`).
- `KNat → Int`-Erasure an jeder kanonischen Grenze, **nie** still zurückgeraten
  (`kind_erase`/`kenv_to_kopts`/`typquant_to_ast`).

## Tests (Code, E3 — nicht ausgeführt)

- `crates/sail-ty/tests/env_coverage.rs`: Endzustand (KindEnv final state,
  val-specs, registers, Namespace-Trennung + Scattered-Freeze, Scope-Pop).
- `crates/sail-ty/tests/env_ast_bridge.rs`: kanonische Sichten (`KOpt`-Teleskop,
  `TypSchm`-Schemes, `KNat`-Erasure).
- `crates/sail-ty/tests/env_serde_roundtrip.rs`: JSON-Stabilität.
- Modultests: `src/env.rs` (CoW, Trennung, Iteration), `src/kind.rs`
  (len/is_empty/remove), `src/ast_env.rs` (Teleskop-Sortierung, Roundtrip).
- Scattered-Seite: `crates/sail-resolve/tests/resolve.rs`
  (`scattered_open_clause_impl_end_roundtrip`, Kollisionstests).

## Done (Lane 111)

- [x] `KindEnv`/`TypeEnv`-Endzustand: val-specs/registers/locals vollständig
      (zähl-/iterier-/löschbar, CoW, serde-stabil).
- [x] Scattered begründet eingefroren (resolver-owned, hier nur geschlossene
      Schemes), dokumentiert in Modul-Docs (`env.rs`, `ast_env.rs`).
- [x] Diese Datei (`ENV_COVERAGE.md`) als Freeze-Protokoll.
- [x] `#![forbid(unsafe_code)]` überall (Modul- + Testdateien).
