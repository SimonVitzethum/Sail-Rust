# Lane-05 OCaml-Mutable-Parität (nur messen/vergleichen) — 2026-10-12

Stand: Worktree `worktrees/lane-05` (Branch `lane-05`, Basis `a972e3a`), keine `crates/**`-Änderung.
OCaml-Referenz: `/home/simon/.opam/sail/bin/sail`, Sail 0.20.3, jeweils `-just_check`.
Limit: je Datei `MemoryMax=256M`, `MemorySwapMax=0`, `WorkingDirectory=$PWD` (Lane-05-Root),
ausschließlich serialisiert über `/tmp/opencode/sail-gate/gated-run.sh 256M ...` (kein Exit 99).

## Herkunft der Fixtures (nur gelesen)

Quelle (nur gelesen, nicht geändert): `/home/simon/Dokumente/Sail/sail-rs/crates/sail-ty/tests/canonical_program.rs`
(dirty `main`-Stand mit `MUTABLE_DECLARATIONS` / `MUTABLE_POSITIVE` / 21 negativen Fällen).
Daraus erzeugt (transient, `target/` ist keine dauerhafte Quelle):

- `target/mutable-reference/positive.sail`: 7 Deklarationen + alle 15 Positivfälle kombiniert
- `target/mutable-reference/per-case/pos-*.sail` (15): je 7 Deklarationen + 1 Positivfall
- `target/mutable-reference/per-case/neg-*.sail` (21): je 7 Deklarationen + 1 Negativfall
- `target/mutable-reference/negative-NN.sail` (21): Kopie der Negativ-Einzelfälle (Kompatibilität)
- `target/mutable-reference/logs-gated/`: rohe Gate-Logs je Datei

Gemeinsame 7 Deklarationen je Datei:

```
register R : int
val opaque_int : unit -> int
val opaque_bool : unit -> bool
val bounded_index : forall 'n, 0 <= 'n <= 2. int('n) -> unit
val at_least_zero : forall 'n, 'n >= 0. int('n) -> unit
val ge_zero : forall 'n. int('n) -> bool('n >= 0)
val only_true : forall ('p : Bool), 'p. bool('p) -> unit
```

## Übersicht

- Positive: **15/15 von OCaml angenommen (Exit 0)**, kombinierte `positive.sail` ebenfalls Exit 0.
- Negative: **21/21 von OCaml abgewiesen (Exit 1)**.
- **Keine Abweichung zur Rust-Seite**: Rust erwartet Annahme für die 15 Positiven und
  Ablehnung für die 21 Negativen (`position >= positive` ⇒ `is_err`); OCaml trifft dieselbe
  Annahme/Ablehnung in allen 36 Einzelfällen.
- Referenzwarnungen ≠ Fehler: `neg-07`/`neg-08` melden zusätzlich `Warning: Redundant type
  annotation`, werden aber korrekt mit Exit 1 abgewiesen. Alle Positiven ohne Warnung.
- Speicher: alle Einzelläufe Peak ca. 11.6–12.7M, kombiniert 12.1M; Swap jeweils 0B; deutlich unter 256M.
- Neu gegenüber altem Stand (13+/19-): positiv `vector_inference`, `let_scope`; negativ
  `let_escape`, `var_escape`. Alle vier neuen Fälle treffen die Rust-Semantik (2× Accept, 2× Reject).

## Tabelle Positive (erwartet: Accept / Exit 0)

| Fall | erwartet | OCaml-Exit | Bemerkung |
| --- | --- | --- | --- |
| `mutate`: `function mutate() -> int = { var x : int = 0; x = 1; x }` | Accept | 0 | Peak 12.1M, keine Warnung |
| `inferred_mutate`: `var x = opaque_int(); x = 7` | Accept | 0 | Peak 12.1M, keine Warnung |
| `inferred_boolean`: `var x = opaque_bool(); x = false` | Accept | 0 | Peak 12.2M, keine Warnung |
| `inferred_constant`: `var x = 0; x = 0` | Accept | 0 | Peak 12.1M, keine Warnung |
| `tuple_mutate`: `var (x, y) = (opaque_int(), true); (x, y) = (1, true)` | Accept | 0 | Peak 12.2M, keine Warnung |
| `write_register`: `R = 3; R` | Accept | 0 | Peak 11.9M, keine Warnung |
| `vector_inference` (neu): `let values = [opaque_int(), 1]` | Accept | 0 | Peak 11.6M, keine Warnung |
| `let_scope` (neu): `{ let x = 1; let y = x; y }` | Accept | 0 | Peak 12.1M, keine Warnung |
| `ascending`: `foreach (i from 0 to 2) bounded_index(i)` | Accept | 0 | Peak 12.5M, keine Warnung |
| `descending`: `foreach (i from 2 downto 0) bounded_index(i)` | Accept | 0 | Peak 12.1M, keine Warnung |
| `bounded_mutate`: `var x; foreach ... { x = i }` | Accept | 0 | Peak 12.2M, keine Warnung |
| `while_loop`: `while true do ()` | Accept | 0 | Peak 12.3M, keine Warnung |
| `repeat_loop`: `repeat () until true` | Accept | 0 | Peak 12.7M, keine Warnung |
| `while_fact`: `while ge_zero(x) do at_least_zero(x)` | Accept | 0 | Peak 12.3M, keine Warnung |
| `mutable_condition`: `var x:bool; while x do { x = false }` | Accept | 0 | Peak 12.2M, keine Warnung |
| kombiniert `positive.sail` (15 Fälle in einer Datei) | Accept | 0 | Peak 12.1M, keine Warnung |

## Tabelle Negative (erwartet: Reject / Exit 1)

| Fall | erwartet | OCaml-Exit | Bemerkung |
| --- | --- | --- | --- |
| `wrong_init`: `var x:int = false` | Reject | 1 | Peak 12.3M; `Type mismatch between int('ex3#) and bool(false)` |
| `let_escape` (neu): `{ { let x = 1; () }; x }` | Reject | 1 | Peak 12.2M; `Identifier x is unbound` |
| `var_escape` (neu): `{ { var x:int = 1; () }; x }` | Reject | 1 | Peak 12.1M; `Identifier x is unbound` |
| `wrong_write`: `var x:int=0; x=false` | Reject | 1 | Peak 12.1M; `Type mismatch between int('ex5#) and bool(false)` |
| `constant_write`: `var x=0; x=1` | Reject | 1 | Peak 12.2M; `int(1) is not a subtype of int(0)` |
| `immutable_write`: `let x=0; x=0` | Reject | 1 | Peak 12.5M; `Cannot modify immutable let-bound constant ... x` |
| `shadow_local`: `function shadow_local(x:int) = { var x:int=0; ...}` | Reject | 1 | Peak 11.8M; `Cannot modify immutable let-bound constant ... x` |
| `shadow_mutable`: `var x; var x` | Reject | 1 | Peak 12.1M; Warnung `Redundant type annotation` + `var expression can only be used to declare new variables` |
| `shadow_register`: `var R:int=0` | Reject | 1 | Peak 12.5M; Warnung `Redundant type annotation` + `var expression can only be used ...` |
| `missing_target`: `missing = 0` | Reject | 1 | Peak 12.2M; `Variable declaration with unclear (or no) scope ...` |
| `wrong_register`: `R = false` | Reject | 1 | Peak 12.1M; `Type mismatch between int('ex3#) and bool(false)` |
| `tuple_write`: `(x,y)=(1,false)` statt `(1,true)` | Reject | 1 | Peak 12.2M; `Failed to prove constraint: ((not(false) \| true) & not(true)) ...` |
| `bad_bounds`: `foreach 0 to 3` vs `0<=n<=2` | Reject | 1 | Peak 12.1M; `Could not resolve quantifiers for bounded_index` |
| `bad_descending`: `foreach 3 downto 0` | Reject | 1 | Peak 11.9M; `Could not resolve quantifiers for bounded_index` |
| `write_iterator`: `foreach ... { i = 1 }` | Reject | 1 | Peak 12.2M; `Cannot modify immutable let-bound constant ... i` |
| `bound_escape`: `foreach...(); bounded_index(i)` | Reject | 1 | Peak 12.4M; `Identifier i is unbound` |
| `wrong_bound_type`: `foreach (i from false to 2)` | Reject | 1 | Peak 12.1M; `Ranges in foreach overlap` |
| `wrong_body_type`: `foreach ... 1` statt `unit` | Reject | 1 | Peak 12.1M; `int(1) is not a subtype of unit` |
| `wrong_condition`: `while 0 do ()` | Reject | 1 | Peak 11.9M; `Type mismatch between bool('ex1#) and int(0)` |
| `until_fact`: `repeat at_least_zero(x) until ge_zero(x)` | Reject | 1 | Peak 12.2M; `Could not resolve quantifiers for at_least_zero` |
| `stale_fact`: `if x then { x=false; only_true(x) }` | Reject | 1 | Peak 12.0M; `Could not resolve quantifiers for only_true` |

## Logs

- `lane-05-ocaml-mutable-positive-256M.log`: kombinierter Positivlauf + alle 15 Einzel-Positivläufe (Gate).
- `lane-05-ocaml-mutable-negative-256M.log`: alle 21 Einzel-Negativläufe (Gate).
- `lane-05-logs/`: 37 rohe Gate-Einzellogs (`positive.sail.log`, `pos-*.sail.log`, `neg-*.sail.log`).
- Fixtures selbst liegen transient unter `target/mutable-reference/` (git-ignoriert), nicht in `evidence/`.

## Bericht: Abweichungen zur Rust-Seite

Keine. Alle 15 positiven Mutable-Fälle werden von OCaml angenommen, alle 21 negativen
abgewiesen — exakt die in `mutable_declarations_assignments_and_loop_bounds_are_checked`
kodierte Erwartung (`is_err ⇔ position >= positive`). Die zwei neuen Positiven
(`vector_inference`, `let_scope`) und zwei neuen Negativen (`let_escape`, `var_escape`)
verhalten sich referenzkonform. Warnungen nur als Zusatz zu Ablehnungen (`shadow_mutable`,
`shadow_register`), keine Warnung dreht eine Annahme/Ablehnung um.
