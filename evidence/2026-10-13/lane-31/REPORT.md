# Lane-31: ARM-Gate-Triage — erste 3 Blocker (nur Diagnose, kein Fix)

Stand: 2026-10-13. Basis: Haupt-Worktree auf `a972e3a` plus uncommittete
Mutable-/Grammatikarbeit (dirty, vgl. `git status`; keine Crate-Änderung durch
diese Lane). OCaml-Referenz: `/home/simon/.opam/sail/bin/sail`, Version 0.20.3.
Schwerer Lauf via `/tmp/opencode/sail-gate/gated-run.sh 16G
scripts/arm-lean-gate.sh`; kleine Referenz-/Repro-Läufe via direktem
`systemd-run` (Gate-Lock war durch parallele Lanes belegt; Limits jeweils
gesetzt, Swap 0 — s. Einzelnachweise unten).

## Gate-Lauf (Vollbeleg in `gate-compiler.log`)

- Modell: `../sail-arm/arm-v9.4-a`, 22 Quellen (`sources.txt`-Auszug im Log),
  Frontend: **31267 Definitionen aus 45 Dateien**, 1018419399 AST-Bytes,
  16 Worker, 30,5 s; cgroup-Peak **1,3 GiB**, Swap 0. Danach Abbruch.
- Erste Abbruchzeile (Deklarationsphase, noch vor jeder Body-Prüfung/Lean):
  `sail-rs: <sail-lib>/concurrency_interface/read_write_v1.sail: declaration
  typecheck: unbound variable ''pa' at Span { file: FileId(15), start: 6288,
  end: 7970 }`, Exit 1.
- Gegenüber dem Stand aus `evidence/2026-10-11/arm-vectors-16G.log`
  (Abbruch `to_bytes_le`, `vector.sail`) ist der Gate-Kopf weitergekommen:
  `to_bytes_le` passiert jetzt die Deklarationsphase; der neue Kopfblocker
  liegt in der Outcome-Deklaration (Blocker B1). Die `to_bytes_le`-Body-Lücke
  besteht als Blocker B2 fort (jede Rust-Fixture mit Prelude bricht dort ab).

## Blocker-Tabelle

| # | Sail-Modell (Datei:Zeile) | OCaml-Referenz (`sail -just_check`) | Vermutete fehlende Regel (Pfad:Zeile) | Status |
|---|---|---|---|---|
| B1 | `sail/lib/concurrency_interface/read_write_v1.sail:142-186` (Outcome `sail_mem_read`, `with 'pa : Type` + inneres `val pa_bits : 'pa -> …` in `:151`; Gate-Span Byte 6288-7970) | `type_check.ml:5379-5404` `check_outcome`: `params` per `add_typquant` + `wf_typschm` ins lokale Env, innere Defs via `check_defs` darin — äußere `with`-Binder sind innen sichtbar. Nachweis: `sail/test/typecheck/pass/outcome_impl.sail:7-11` (`val to_bits : 'a -> bits(64)` unter äußerem `'a: Type`) wird **akzeptiert**; Kopie als `fixture-b1-outcome.sail` | `crates/sail-ty/src/program.rs:868-884`: innere Outcome-`val`-Schemes werden mit `check_scheme` (nur eigene Quantoren, `program.rs:1150-1159`) geprüft, ohne die äußeren Outcome-Quantoren (`params` + `typschm.quant`, vgl. `:852-855`) in den Scope zu nehmen → `unbound ''pa`. Naheliegender Fix-Ort: äußeren Scope wie in `check_annotation` (`program.rs:1224-1255`) einspeisen | **laufzeit-betätigt**: Gate-Abbruch + Minimal-Repro `fixture-b1-outcome.sail` → Rust `declaration typecheck: unbound variable ''a'` (Span 60-180 = Z. 7-12), OCaml Erfolg |
| B2 | `sail/lib/vector.sail:478` (`res[i] = b[(8 * i + 7) .. (8 * i)]` im `_DEFAULT_DEC`-Zweig von `to_bytes_le`, `:475-481`); Geschwister `:486`, `from_bytes_le` `:498`/`:506` (Slice-LHS `res[a..b] = …`) | `type_check.ml:3575ff` `infer_lexp`, `LE_vector` (`:3613ff`)/`LE_vector_range` (`:2033ff`): Wurzel muss mutable/Register sein, Index voll geprüft, Bounds `0 <= i < len` (ordnungsabhängige Slice-Bounds) per Solver bewiesen. Nachweis: `fixture-b2-vector-assign.sail` (`res[0] = x`) wird **akzeptiert** | `crates/sail-ty/src/canonical_body.rs:912-917` (`assignment_type`): `LExpKind::Vector`/`VectorRange`/`Field` werden explizit abgewiesen (`"field, vector or reference assignment target"`); nur `Id`/`Typ`/`Tuple` (`:891-911`) existieren | **laufzeit-betätigt**: jede Rust-Fixture mit Prelude (`fixture-b2-…`, `-b3-…`, `-match`) bricht in `vector.sail` ab: `body typecheck: typechecking incomplete at file4:15050-15221: field, vector or reference assignment target` |
| B3 | `sail/lib/concurrency_interface/read_write_v1.sail:111-115` (`match request.access_kind { AK_explicit(eak) => match eak.variety …` in `mem_read_request_is_exclusive`); ebenso `:124`, `:206-211`, `:230`, `:246`, ARM-weit üblich | `type_check.ml:2176-2195` `check_exp` `E_match` (Scrutinee inferieren/checken, Cases via `check_case`, Vollständigkeit via `PC.is_complete_wildcarded`). Nachweis: `fixture-b3-match.sail` wird **akzeptiert** | `crates/sail-ty/src/canonical_body.rs`: **kein** `ExpKind::Match`-Arm (Check-Arme `:1601-1908`, Infer `:1263-…`; einziger Match-Bezug ist die PExp-Zerlegung `:1190-1191`) → fällt in Catch-all `:1909-1914` (`"expression rule is not yet implemented in the canonical checker"`) | **Nachfolger, statisch bestätigt**: zur Laufzeit noch von B2 maskiert (Gate erreicht match-tragende Bodies nicht; Rust-`match`-Fixture bricht ebenfalls mit B2-Meldung ab). Fällt sofort nach B2-Fix auf |

## Detaillogik je Blocker (kurz)

- **B1**: Der Outcome-Kopf (`params` = `with`-Binder, `typschm.quant`) wird in
  `program.rs:852-855` wohlgeformt geprüft, aber beim Prüfen der inneren
  `val`-Schemes (`:868-884`) nicht als `outer`-Scope übergeben. Exakt das
  Muster aus `outcome_impl.sail` (inneres `val` nutzt äußeres `'a`) löst den
  Gate-Fehler im Modell aus (`'pa` in `pa_bits`, `read_write_v1.sail:151`).
  Zweites Outcome `sail_mem_write` (`:216-…`, identisches Muster) wäre der
  unmittelbar nächste B1-Fall.
- **B2**: `res[i] = …` ist im Rust-AST kein Funktionsaufruf, sondern
  `LExpKind::Vector`-L-Wert (`sail-parse/src/convert.rs:805`); die
  `vector_*#`-Normalisierung (`sail-ty/src/vector_prims.rs:168-192`, nur
  Namen→Overload) greift dort nicht — sie betrifft nur die Lese-Seite
  (`b[…]` → `vector_subrange#`, `convert.rs:1160-1161`). Die L-Wert-Seite
  braucht `infer_lexp`-Bounds; das ist exakt das Thema des laufenden
  Lane-30-Adapters (`vector_lexp.rs`/`lexp_bounds.rs`).
- **B3**: Nach B1+B2 folgen im selben Modell-File `match`-Ausdrücke
  (s. Zeilen oben) sowie `request.pa`-Field-Zugriffe (`E_field`,
  Lane-25-Gebiet) und `vector_subrange#`-Bounds ohne Solver-Beweise im
  `App`-Arm (`canonical_body.rs:1666-1697` beweist keine Index-Bounds;
  `vector_prims.rs` normalisiert nur Namen — Soundness-Lücke, aber kein
  Abbruch; Lane-27-Gebiet).

## Empfehlung: welche Lanes (25-30) lösen was

(Worktree-Themen nach deren neuen Dateien auf `a972e3a` bestimmt.)

- **B1 → keine der Lanes 25-30 besitzt das Thema.** `lexp_bounds`/`vector_lexp`
  (30), `exp_match` (26), `exp_struct` (25), `exp_misc` (27),
  `mapping_expand` (29), `measure_check` (28) decken Outcome-`with`-Scopes
  nicht ab. Empfehlung: **eigene Folge-Lane für Outcome-Deklaration**
  (`program.rs:839-886`, äußere Quantoren in innere `val`/`impl`-Scopes;
  danach `impl`-Bodies + `instantiation`, `type_check.ml:5423ff`), weil B1
  die gesamte Body-/Lean-Pipeline blockiert. Notfalls an Lane-29 andocken
  (Deklarations-/Overload-Umfeld), aber getrennte Lane ist sauberer.
- **B2 → Lane-30** (`vector_lexp.rs` + `lexp_bounds.rs`: `LE_vector` /
  `LE_vector_range` mit mutable-/Register-Wurzel, Indexprüfung,
  `0 <= i < len` + ordnungsabhängige Slice-Bounds, `WORK_STATE.md` Punkte
  4-5). Erster Body-Blocker nach B1-Fix.
- **B3 → Lane-26** (`exp_match.rs`: `E_match` check + infer inkl.
  `check_case`/Vollständigkeit, `type_check.ml:2176ff`). Sofort fällig nach
  B2, belegt im selben Modell-File.
- Flankierend danach: **Lane-25** (`E_field`/`E_struct`, `request.pa` u.a.),
  **Lane-27** (`vector_subrange#`/`vector_access#`-Boundsbeweise im App-Arm),
  **Lane-28** (`termination_measures.sail`, Gate-Ende), **Lane-29**
  (Mappings/scattered, `map_clauses.sail`).

## Dateien dieser Lane (nur `evidence/2026-10-13/lane-31/`)

- `gate-compiler.log` — Kopie des Gate-Compiler-Logs
  (`target/arm-lean-gate/run.mkQRImAM/compiler.log`), inkl. Frontend-Zählung
  und B1-Abbruchzeile.
- `fixture-b1-outcome.sail` — OCaml-passende Outcome-Minimalstelle
  (Kopie von `sail/test/typecheck/pass/outcome_impl.sail`); Rust: `unbound
  ''a'`, OCaml: Erfolg.
- `fixture-b2-vector-assign.sail` — `res[0] = x`; OCaml: Erfolg; Rust:
  B2-Meldung in `vector.sail`.
- `fixture-b3-bitslice.sail` (+ `fixture-b3b/c-slice/vecread-plain`) —
  Vorversuche zur Lese-Seite (teilweise verworfen: ohne Prelude fehlen
  `bits`/`vector`-Typen auf beiden Compilern); B3-Nachweis läuft über
  `fixture-b3-match.sail`.
- `fixture-b3-match.sail` — `match`-Minimalstelle; OCaml: Erfolg; Rust:
  derzeit B2-maskiert (belegt Nachfolger-Status).
- `REPORT.md` (diese Datei).
