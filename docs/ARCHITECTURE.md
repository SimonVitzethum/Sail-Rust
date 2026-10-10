# sail-rs — Architektur (Lane 53)

Stand: Worktree `lane-53` (Branch `lane/53-arch-docs`), 35 Workspace-Member.
Quelle: reine Read-only-Analyse (`src/lib.rs`-Docs, `Cargo.toml`, Lane-Specs);
E3-Test-Moratorium: keine Builds/Tests ausgeführt.

Review-Verweis (Done-Kriterium): Lesung durch Lane 47 (`lanes/47-ast-unify.md`)
/ Lane 48 (`lanes/48-workspace-unify.md`) genügt — beide Specs sind im Worktree
unter `lanes/` vorhanden und unten als Owner referenziert.

## 1. Pipeline-Übersicht (Parse → Lean)

```text
.sail-Quellen (+ vendored Prelude, sail-prelude)
        │
        ▼
sail-lex (16) ── Tokenstrom ──▶ sail-parse (02) ──▶ ParsedFile (Def-Liste, file_start/file_end)
        │                              │
        │                              ▼ (Alternative, noch Skelett)
        │                       sail-grammar (17): lexer + AST + sail.lalrpop,
        │                              generierter Parser noch nicht verdrahtet
        ▼
sail-resolve (03) ── Symboltabellen + Ctor-Renames ──▶ sail-ast (01, Datentypen)
        │
        ▼
sail-nexp (04, NExp-Algebra, rein syntaktisch)
        │
        ▼
sail-ty (05, bidirektionales Checken) ◀── sail-solver (06, Trait + Memo + Pool, opt. Z3)
        │
        ▼
sail-rewrite (07, shard-lokale Lean-Rewrite-Pipeline, rayon)
        │
        ▼
sail-effects (08, Callgraph + Effekt-Labels: Pure/Stateful/EarlyReturn/Loop)
        │
        ├── sail-loops (27, while/until/foreach + Fuel-Varianten, Idiom-Rendering)
        ├── sail-except (28, early_return/throw/try/exit/assert → SailM/SailME-Mapping)
        ├── sail-termination (26, termination_by/decreasing_trivial, Fuel-Sigs)
        ├── sail-mapping (20, realize_mappings + encdec-Split-Skizze)
        ├── sail-patterns (21, Exhaustiveness/Redundanz + match_bv-Erkennung)
        ├── sail-bitvec (22, E_vector/P_vector/L_bin-hex, Breitenprüfung)
        ├── sail-int (23, Int/Nat/Range/Atom + E_for-Lowering)
        ├── sail-real (24, FakeReal-Default vs. mathlib-Opt-in, L_real-Literale)
        ├── sail-extern (25, extern-"lean"-FFI-Tabelle, harter Fehler statt sorry)
        ├── sail-regs (19, Register-Extrakt + Register/RegisterType/regstate-Snippets)
        └── sail-archsem (29, Concurrency-Spiegel V1/V2 — kein eigenes Modell, E1)
        │
        ▼
sail-lower (09, Mono + ANF, shard-weise streaming)
        │
        ▼
sail-lean (10, streaming Lean-Printer, O(grösste Def), parallel)
        │
        ├── sail-lake (12, Lake-Projekt-Generator: lakefile.toml + Support-Lib)
        └── sail-exe (34, main-Stub + --lean-executable-Gate, speist LakeOptions.executable)
```

Quer dazu (kein Datenfluss-Glied, sondern Infrastruktur):

```text
sail-shard (11, STUB) ......... Shard-Grenzen + Fingerprints + Scheduler (Owner Lane 11, aktiv)
sail-sysregs-shard (31) ....... sysregs_autogen.sail als isolierter Shard (Streaming-Split, FNV-1a)
sail-sve-sme (32) ............. instrs64_sve/sme-Shards (vendored sail-parse-Scanner + Pattern-Katalog)
sail-instrs-core (33) ......... instrs32/64 + v8_base Kern-Analyse (Scanner/Plan/Dispatch/Measure)
sail-cache (36) ............... Disk-Cache target/sail-cache/<hash>/ (atomar, LRU-GC)
sail-oracle (13) .............. Difftest-Harness: OCaml-sail als Orakel vs. Rust-Output
sail-perf (14) ................ RAM/Timing-Infra (GNU-time-Parser, JSONL-Phasen, Budgets)
sail-diag (35) ................ Shared SailError + --explain-Katalog E3501–E3523 + ExitCodes
sail-cli (38, STUB) ........... Drop-in-sail-CLI (Owner Lane 38, queued)
```

## 2. Crate-Verantwortlichkeiten + Status

Legende: **fertig** = implementiert mit Tests, dieser Lane-Done-Stand;
**Skelett** = API-Doku steht, Kernfunktion fehlt dokumentiert;
**Stub** = Lane-15-Platzhalter, Owner-Lane liefert echt;
**Duplikat** = bewusst vendored, Lane 47/48 führt zusammen.

| Crate | Lane | Verantwortung (OCaml-Referenz) | Status |
|---|---|---|---|
| sail-ast | 01 | Kern-AST: `Node<K>` + `NodeId` + `Span`, `Symbol`-Interner, JSON-Roundtrip | fertig |
| sail-parse | 02 | Definitions-Parser (1 Thread/Datei, `parse_files_parallel`), Roh-Bodies + Spans | fertig |
| sail-resolve | 03 | Scope-Resolution: `Env`-Tabellen + `compute_ctor_renames` (pur) | fertig |
| sail-nexp | 04 | NExp/nconstraint-Algebra: Simplify, syntaktische `provably_*`-Prädikate, Lean Nat/Int-Mapping (kein Solver) | fertig |
| sail-ty | 05 | Kind-Check, `TypQuant`, bidirektionales Infer/Check, `Solver`-Trait + `StubSolver` | fertig |
| sail-solver | 06 | `Solver`-Trait, LRU-`MemoCache` + `MemoSolver`, exklusiver `SolverPool`, `StubSolver`-Default; `z3`-Feature optional | fertig |
| sail-rewrite | 07 | Shard-lokale Lean-Rewrites (`lean_rewrites`-Spiegel), Pipeline über rayon | fertig |
| sail-effects | 08 | Callgraph (u32-Index-Maps) + Effekt-Labels für Lean-Monadenwahl | fertig |
| sail-lower | 09 | Monomorphisierung (`-auto_mono`-Äquivalent) + ANF (`internal_lets`, `remove_blocks`), shard-weise | fertig |
| sail-lean | 10 | Streaming-Lean-Printer (`fmt::Write`-Sink, `Send+Sync+Clone`, parallel), Monad-Tabelle Pure→`Ret`/Stateful→`SailM`/EarlyReturn→`SailME` | fertig |
| sail-lake | 12 | Lake-Projekt-Gen (`start_lean_output`-Port): Toolchain-Pin v4.29.0, lean-sail v6, lakefile.toml inkl. `[[lean_exe]]`-Branch | fertig |
| sail-oracle | 13 | Difftest: OCaml-`sail` als Orakel, Normalizer + Diff + Report; Rust-Seite Stub bis Backend steht | fertig (Orakel-Seite; Rust-Seite Skelett per Design) |
| sail-perf | 14 | `time -v`-Parser, JSONL-Phasenlog (`-dprofile`-Äquivalent), Budgets 4/16/24 GiB | fertig |
| sail-lex | 16 | Standalone-Lexer: nie fallend (Recovery-Fehler im `LexResult`), minimale lokale `Span`/`FileId` | fertig |
| sail-grammar | 17 | Menhir→LALRPOP-Port: skelettaler AST + Hand-Lexer + `sail.lalrpop` als Doku-Artefakt; `parse_*` lexen, melden dann `SKELETON_MSG` (LALR-Konflikte, siehe `GRAMMAR.md`) | Skelett (aktiv, Owner Lane 17) |
| sail-prelude | 18 | Vendored `sail/lib/*.sail` (26 Dateien, Sail 0.20.3) via `include_str!`, + Empty-Model-Skelett | fertig |
| sail-regs | 19 | `find_registers`-Äquivalent: Extrakt + `Register`-Induktiv + `RegisterType`-Abbrev + `regstate` (flach/gruppiert) + `reset.sail`-Defaults | fertig |
| sail-mapping | 20 | `realize_mappings` + `mapping_patterns` + `untranslatable_mapping` (Exit→`throw Error.Exit`) + encdec/execute-Split-Skizze; Fixture `mapping.sail` als Rust-Tests | fertig |
| sail-patterns | 21 | `pattern_check`-Äquivalent (redundant entfernen + Wildcard-Catch-all) + `is_match_bv`/`is_bitvector_pattern` | fertig |
| sail-bitvec | 22 | `E_vector`→`BitVec.join1`/`#v`, `E_vector_append`→`+++`, `P_vector(_concat)`, `L_bin`/`L_hex`, `bitfield`→`extractLsb`/`updateSubrange`, Breitenfehler mit Span | fertig |
| sail-int | 23 | `int`→`Int`/`nat`→`Nat`/`atom`/`range` via `provably_nneg`, `op_of_id`-Tabelle, `E_for`→`foreach_Z_up/down` | fertig |
| sail-real | 24 | `--lean-real-numbers`: Default `FakeReal` (`abbrev real := Float`), Opt-in `Real.lean` + mathlib-Require; `L_real` als rationale Strings | fertig |
| sail-extern | 25 | `op_of_id`-Operatoren, `DEF_fundef`-extern-Suppression, fehlendes `lean`-Mapping = harter Fehler (E2, nie `sorry`) | fertig |
| sail-termination | 26 | `termination_by (…).toNat` (`doc_termination`), `whileFuelM`/`untilFuelM`-Sigs, `move_termination_measures`-Skizze | fertig |
| sail-loops | 27 | `while#`/`until#`/`foreach#` + Fuel-Varianten: `LoopKind`/`Order`/`LoopVars`, `Id.run`-vs-`do`-Effektentscheidung, Idiom-Rendering | fertig |
| sail-except | 28 | `early_return`/`E_exit`/`E_throw`/`E_try`/`E_assert`-Mapping + Funktions-Frame (`SailME.run do`/`do`/`Id.run do`/`ExceptM.run do`) | fertig |
| sail-archsem | 29 | Concurrency-**Spiegel** V1/V2 (E1: Lean Master, Rust Spiegel; Feld-für-Feld-Verweise auf `ArchSem*.lean` + `pretty_print_lean.ml:1524-1605`), Difftest-Vektoren in `tests.rs` | fertig |
| sail-sysregs-shard | 31 | `sysregs_autogen.sail`-Shard: Streaming-Split (2815 val/function-Paare), Blöcke à 64 Defs, Paarungsprüfung + FNV-1a, Single-Bin + Lib | fertig |
| sail-sve-sme | 32 | SVE/SME-Shards: Def-Split + Vektor-Pattern-Katalog + `Fail`-Liste mit Spans; Scanner aus sail-parse **vendored (byte-identisch, dokumentiert)** | fertig (Duplikat dokumentiert, Owner Lane 32; Unify → Lane 47/48) |
| sail-instrs-core | 33 | Kern-Shard-Analyse: Streaming-Scanner, Shard-Plan (exec/decode/dispatch/helper), Decode→Execute-Kanten + SEE-Wächter, Rumpf-Maße; Zahlen in `CORE_REPORT.md` | fertig |
| sail-exe | 34 | `main_function_stub`-Port (`main`→`sail_main`, `def main … main_of_sail_main …`), `--lean-executable`-Gate (Stub + `[[lean_exe]]` nur mit Flag **und** gesehenem `main`, E2 ohne `sorry`) | fertig |
| sail-diag | 35 | Shared `SailError` (`span/pass/code/message/hint`), `--explain`-Katalog `E3501–E3523` (23 `pretty_print_lean.ml`-Sites), `ExitCode`; null Dependencies | fertig |
| sail-cache | 36 | Disk-Cache `<root>/<hash>/` (`output` + `fingerprint`), atomar (Tempfile+fsync+Rename), LRU-`gc`, Zweitlauf schreibt nichts | fertig |
| sail-shard | 11 | Shard-Grenzen + Fingerprints + Scheduler | **Stub** (Lane-15-Platzhalter; Owner Lane 11, aktiv — NICHT anfassen) |
| sail-cli | 38 | Drop-in-`sail`-CLI (`--lean`, `-o`, `-dprofile`, Exit-Codes) | **Stub** (Lane-15-Platzhalter; Owner Lane 38, queued) |

## 3. Bekannte Abweichungen / Tech-Debt (zur Kenntnis, kein Umbau in dieser Lane)

- **Lokale Minimaltypen statt `sail-ast`:** fast jede Frontend-Crate (`sail-parse/mini_ast`,
  `sail-ty`, `sail-resolve`, `sail-patterns`, …) führt eigene `Span`/`Def`/`Pat`-Minimale
  mit `*_NEEDED.md`-Swap-Vertrag. Unify-Owner: Lane 47 (ein AST) + Lane 48 (Workspace).
- **`sail-sve-sme` vendored `sail-parse`-Scanner** (Absicht, byte-identisch, Diff-freundlich).
  Zusammenführen erst in Lane 47/48.
- **`sail-grammar` ist Skelett** (Owner Lane 17, aktiv): `parse_*` → `SKELETON_MSG`.
  Produktivpfad bleibt `sail-lex` → `sail-parse`.
- **`sail-exe` Dev-Dep-Pfad** (`crates/sail-exe/Cargo.toml:13`):
  `path = "../../../../crates/sail-lake"` löst vom Workspace aus nicht auf
  (erwartet wäre `../sail-lake`). Ungeprüft wegen E3; Owner Lane 34/48 zur Prüfung.
- **Lints:** `sail-except` hat keinen `[lints]`-Abschnitt; `sail-grammar` und
  `sail-sysregs-shard` pinnen eigene `[lints.*]` statt `workspace = true`.
  Vereinheitlichen in Lane 48.
- **Inter-Crate-Deps** sind derzeit fast null (nur `sail-exe`→`sail-lake` als Dev-Dep für
  einen Docking-Test); die Verdrahtung der Pipeline zu einem Treiber (CLI, Lane 38)
  steht noch aus.

## 4. Bindende Entscheidungen (Kurzfassung, Details in `DECISIONS.md`)

- **E1 — Lean ist Master** fürs Speichermodell (mit Beweisen), Rust (`sail-archsem`) nur
  Spiegel; Drift = roter Difftest = Merge-Blocker. Änderungsfluss immer zuerst Lean.
- **E2 — Nirgends `sorry`**: weder generierter Code noch Templates noch Support-Lib;
  lieber harter Fehler. CI-Gate per `grep`.
- **E3 — Test-Moratorium**: keine Builds/Tests in Lane-Agents (OOM-Schutz, kein Swap);
  Verifikation per Review + zentral durch Integrator. Auch diese Lane hat nichts gebaut.
