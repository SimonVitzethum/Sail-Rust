# NExp/Constraint-Operator-Tabelle (Lane 80 — solver-complete)

Quelle: `Nexp_*`/`n_constraint` in `sail/src/lib/extraction/Ast.ml`;
Umsetzung: `sail-nexp` (`expr`/`simplify`/`pred`/`lean`), `sail-solver`
(`query`, `z3_backend` hinter Feature `z3`).

Legende:

- **exakt**: volstaendige Abbildung, beweist alles Beweisbare in der Theorie.
- **konservativ**: antwortet nur bei syntaktischer Evidenz (`false` = unbekannt),
  luegt nie.
- **frisch** (Z3): frische uninterpretierte Konstante pro Vorkommen — beweisgesund
  (eine freie Variable staerkt die zu beweisende Formel), verliert nur Praezision
  Richtung `Unknown`.
- **interniert** (Z3): gleicher Name = gleiche Konstante (`new_const`).

## NExp (`Nexp_*`)

| Operator (OCaml) | `sail-nexp` | `eval` | `simplify` | Sign/`provably_*` | Lean (`Nat`/`Int`) | Z3 (`z3` 0.21, `Int`) |
|---|---|---|---|---|---|---|
| `Nexp_id f` | `NExp::Id` | `None` | opaque | `Unknown` | `Int` | frisch (`fresh_const`) |
| `Nexp_var kid` | `NExp::Var` | env | subst | via `Facts` | `Nat` gdw. `provably_nneg` | interniert (`new_const`) |
| `Nexp_constant i` | `NExp::Constant(i128)` | `Some` | Faltung (checked) | Vorzeichenliteral | `Nat` gdw. `>= 0` | exakt (`from_i64`/`from_u64`/Horner Basis `2^32` f. Rest) |
| `Nexp_app ("div",…)` | `NExp::div` | trunkierend (Rust `/`), `None` bei `/0`/Overflow | Faltung; `x/1→x`; `0/d→0` (nur `d≠0` bekannt) | `d>0` const: Zeichen von `a`, sonst `Unknown` | via Sign | frisch (Sail trunkiert, Z3-`div`/`rem` weichen bei negativen Operanden ab) |
| `Nexp_app ("mod",…)` | `NExp::modulo` | trunkierend (Rust `%`) | Faltung; `x%1→0`; `0%d→0` (nur `d≠0`) | `m>0` const: `Nneg`, sonst `Unknown` | via Sign | frisch (Begruendung wie `div`) |
| `Nexp_app ("abs",…)` | `NExp::abs` | `checked_abs` | Faltung; idempotent; `abs(2^x)=2^x` | immer `Nneg` | immer `Nat` | exakt (`x>=0`-`ite`) |
| `Nexp_app (f,…)` sonst | `NExp::App(f, args)` | `None` | Argumente | `Unknown` | `Int` | frisch |
| `Nexp_if (c,t,e)` | `NExp::If` | Zweig per `c.eval` | `True`/`False`-Zweige; `t==e→t` | `t`,`e` gleiches Zeichen → das, sonst `Unknown` | via Sign | exakt (`ite`) |
| `Nexp_times` | `NExp::Times` | `checked_mul` | Faltung; `0`- annihilation; `1`-Identitaet; `(-1)*x→-x`; `(-a)*(-b)→a*b` | Vorzeichentafel (`0*unbekannt→0`) | via Sign | exakt (`Int::mul`) |
| `Nexp_sum` | `NExp::Sum` | `checked_add` | Faltung; `0`-Identitaet; `-(a)+-(b)`, `x+(-y)→x-y` | Vorzeichentafel | via Sign | exakt (`Int::add`) |
| `Nexp_minus` | `NExp::Minus` | `checked_sub` | Faltung; `a-a→0`; `x-0→x`; `0-x→-x`; `(x+c1)-c2` | als `a + (-b)` | via Sign | exakt (`Int::sub`) |
| `Nexp_exp x` (`2^x`) | `NExp::Exp` | `1<<x` f. `x` in `[0,127)`, sonst `None` | `2^0→1`; const in `[0,127)` falten | immer `Pos` | immer `Nat` | exakt f. const `c` in `[0,127)` (Literal, gleiche Domaene wie `eval`), sonst frisch (`power` lieferte `Real`) |
| `Nexp_neg x` | `NExp::Neg` | `checked_neg` | Faltung; `--x→x`; `-(a-b)→b-a` | Zeichen kehrt sich um | via Sign | exakt (`unary_minus`) |

`Nexp_aux` ist nur ein Locations-Wrapper und wird transparent abgebildet
(kein eigener Operator).

## NConstraint (`NC_*`)

| Constraint (OCaml) | `sail-nexp` | `eval` | `simplify`/`normalize` | `Facts::from_constraint` | Z3 (`Bool`) |
|---|---|---|---|---|---|
| `NC_true`/`NC_false` | `True`/`False` | `Some` | absorbieren | ignoriert | `from_bool` |
| `NC_id f` | `Id` | `None` | opaque | ignoriert | frisch (`new_const`, Bool) |
| `NC_var kid` | `Var` (+`subst`-Faltung `0→False`, sonst `→True`) | `None` | opaque | ignoriert | frisch (`new_const`, Bool) |
| `NC_equal (t1,t2)` | `Equal(NExp,NExp)` (nur `A_nexp`-Operanden) | Vergleich | `a==b→True`; const-Faltung | `x==c` setzt beide Bounds | exakt (`Ast::eq`) |
| `NC_not_equal` | `NotEqual` | Vergleich | `a==b→False`; const-Faltung | `x!=0` → `nonzero` | exakt (`eq(...).not()`) |
| `NC_ge/gt/le/lt` | `Ge/Gt/Le/Lt` | Vergleich | `a cmp a` konstant; const-Faltung | var-vs-const Bounds (strikt `±1`, saturierend) | exakt (`ge/gt/le/lt`) |
| `NC_set (n, vs)` | `Set` | `vs.contains` | const-Membership; `[]→False`; sort/dedup | einelementig → beide Bounds | exakt (`Or` von `eq`-Literalen; `[]→False`) |
| `NC_and`/`NC_or` | `And`/`Or` (binaer) | `&&`/`\|\|` | `True`/`False`-Absorption; `x==x`-Dedup; **`normalize`: flatten + sort (kanonisch)** | `And` rekursiv | exakt (`Bool::and`/`or`; leeres `And→True`, leeres `Or→False`) |
| `NC_app (f, args)` | **`App(String, Vec<NExp>)` (Lane 80 neu)** — nur `A_nexp`-Args; fremde `typ_arg`-Sorten muss der Aufrufer abstrahieren | `None` | nur Argumente | ignoriert | frisch (`fresh_const`) |
| `NC_aux` | transparent (kein Operator) | — | — | — | — |

## `provably_*`-Familie (`sail_nexp::pred`, rein syntaktisch, konservativ)

| Praedikat | Bedeutung | Wege zu `true` |
|---|---|---|
| `provably_nneg[_in]` | `x >= 0` | Literal `>=0`; `lower>=0`; `0<=x` via Arithmetik-Tafel; `abs`; `x%m` (`m>0`); `2^x`; `If`-Konsens |
| `provably_pos[_in]` | `x > 0` | Literal `>0`; `lower>0`; `2^x`; Summen/Produkte positiver Formen |
| `provably_nonpos[_in]` | `x <= 0` | dual zu `nneg` (Negation kehrt um) |
| `provably_neg[_in]` | `x < 0` | dual zu `pos` |
| `provably_zero[_in]` | `x == 0` | Literal `0`; `0 * x`; `x - x`; beidseitig gepinnte Bounds `0` |
| `provably_nonzero[_in]` | `x != 0` | `pos`/`neg`-Zeichen; `Facts::with_nonzero`; `from_constraint`-`!=0` |

Alle haben eine kontextfreie (`provably_*`) und eine `Facts`-Variante
(`provably_*_in`). Die Familie ist Obermenge des Lean-Backends (dort nur
`provably_nneg` in `pretty_print_lean.ml`); `int_kind[_in]`/`lean_type_name[_in]`
nutzen exakt die Lean-Regel (`Nat` gdw. `provably_nneg`).

## Z3-Mapping hinter Feature-Flag

- Kein Default: `sail-solver` ohne `z3` baut ohne C-Dep (`StubSolver`).
- Mit `--features z3`: `Z3Solver::prove` (opake `Constraint`s) plus
  `Z3Solver::prove_nconstraint` (strukturiert, `nexp_to_int` /
  `nconstraint_to_bool` direkt aus `sail_nexp`).
- Soundness-Richtungen: frische Konstanten verlieren nur Praezision
  (`Unknown`), luegen nie `Valid`/`Invalid` (freie Variablen staerken die
  Beweislast per Instanziierung, schwaechen die Widerlegung nicht).
- Timeouts: `SolverQuery::timeout_ms` schlaegt `Z3Solver::timeout_ms`;
  umgesetzt via `Params("timeout")`; jede Anfrage terminiert.

## Bounded-LRU (Pflicht)

`MemoCache` ist immer begrenzt: `capacity >= 1` erzwungen (0 wird auf 1
geklemmt), LRU-Eviction bei vollem Cache, TTL-Ablauf zaehlt als Miss,
`evict_expired`/`clear`/`approx_bytes`/`stats` fuer Betrieb und
RAM-Messungen. Unbegrenzte Caches (OCaml `-memo_z3`) sind nicht
konstruierbar: jeder Konstruktor nimmt `SolverCacheConfig`.
