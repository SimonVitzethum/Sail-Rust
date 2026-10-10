# Lane 45 — Fail-Liste E2E Tiny-Arm (Ziel: `sail/test/lean/SailTinyArm.sail`)

Stand: Code-Skizze in `crates/sail-cli` (`src/e2e.rs`, `src/main.rs`).
E3: kein `cargo`/`lake`-Lauf in dieser Lane; Verifikation (u. a. `lake build`)
später zentral beim Integrator. E2: nirgends `sorry` — offene Bodies werden
als harte Fails geführt. E1: Concurrency nur als Durchreiche, kein Zweitmodell.

## Verdrahtet (gegen Main-Crates gelesen, nichts Fremdes kopiert/fixiert)

- `sail_parse::parse_file` → Def-Zensus (Fundef/Val/Mapdef-Namen, Pragma-Includes)
- `sail_lean::LeanPrinter` → Prelude + Stub-Signaturen (`Effect::Stateful/Pure`-Heuristik)
- `sail_regs::{parse_register_lines, extract_registers, lean_of_sail_typ}` → Register-Einträge
- `sail_lake::LakeProject::layout` → Projekt-Skelett + `Defs.lean.sketch`
- `sail_prelude::get("prelude.sail")` → Existenz-Sonde

## Fails (je max. 1 Owner, kein Fixieren fremder Crates)

| ID | Stufe | Beleg (TinyArm-Zeile) | Owner | Notiz |
|----|-------|----------------------|-------|-------|
| F1 | includes | Z. 5, 748–751 (`concurrency_interface*.sail`) | lane-38 | `--sail-libdir`-Auflösung im Treiber fehlt; nur `prelude.sail` via sail-prelude |
| F2 | parse-full-grammar | Z. 599/658/687/703 (Bit-Patterns `@`), 590/594 (scattered), 757–837 (`instantiation`) | lane-17 | Lane-02-Skelett reicht nicht; Grammatik-Hoheit aktiv |
| F3 | ast-unify | `body_src`-Rohtext überall | lane-47 | blockiert Resolver, Ty-Checker, Solver, Effects, Lowering, Mapping |
| F4 | lean-typedefs | Z. 15–25 (bitfield), 67ff (enum), 80ff (struct), 580–586 (union) | lane-30 | `doc_typdef`-Port + deriving/open-Emits |
| F5 | lean-bitvec-exprs | Z. 616–746 (`^`, `+`, `truncate`, `sail_*_extend`) | lane-22 | Ausdruckdruck nach Bitvektor-Semantik |
| F6 | lean-int-arith | Z. 486 (`range(0,31)`), 603–605, 708–710 | lane-23 | Int/Nat-Abbildung + Bereichsnachweise |
| F7 | lean-patterns | Z. 742–745 (`match` auf `option(ast)`), 574/579 (Overloads), scattered-Dispatch | lane-21 | Pattern-Elaboration (`match`/`match_bv`) + Exhaustiveness |
| F8 | lean-effects | Z. 524–577/616–653 (Register), 745/771–799 (`exit`/`assert`) | lane-28 | SailM/SailME-Schachtelung, Exit-Abbildung |
| F9 | lean-concurrency | Z. 757–837 (`sail_mem_*`, `sail_barrier`, `sail_cache_op`, `sail_tlbi`, exceptions) | lane-29 | `doc_instantiations`-Port + Arch-Instanz (E1 beachten) |
| F10 | diagnostics | alle obigen | lane-35 | Span+Pass+Snippet, Exit-Codes, `--explain` |

## Manuell (Integrator, nicht diese Lane)

- `sail-rs --e2e sail/test/lean/SailTinyArm.sail --out-dir <dir>` laufen lassen,
  Fail-Liste prüfen, nach Lane-Fixes `lake build` im out-dir.
