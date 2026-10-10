# E2E_READINESS — Tiny-Arm startklar? (Lane 116, e2e-prep)

Stand: 10.10.2026 | Methode: nur gelesen + als Code verdrahtet (E3 — KEINE Läufe:
kein `cargo build/test/run`, kein `lake build`; Verifikation läuft zentral beim
Integrator/Test-Rig). `#![forbid(unsafe_code)]` gilt für allen neuen Code
(Workspace-Lint `unsafe_code = "forbid"`).

Quelle der Fail-Liste: `lanes/45-e2e-tiny-arm-fails.md` (F1–F10, je max. 1 Owner).
Owner-Status gegen `lanes/*.md` im selben Worktree gelesen.

## 1. Treiber-Einstiege — verdrahtet (Code liegt, Rig-Lauf ausstehend)

| Einstieg | Lage | Stand |
|----------|------|-------|
| `e2e::run(&E2eArgs) -> Outcome` | `crates/sail-cli/src/e2e.rs` | 🟡 Code seit Lane 45/62, jetzt via CLI erreichbar |
| `e2e::run_with_ooc(&E2eArgs, &mut OocDriver<H>) -> Outcome` | `crates/sail-cli/src/e2e.rs` | 🟡 file-backed OoC-Bindung, Rig ausstehend |
| `e2e::E2eArgs::with_input` + `e2e::exit_code_for` | `crates/sail-cli/src/e2e.rs` | 🟢 neu in Lane 116 (0/1/2-Abbildung) |
| `sail-rs --e2e <in.sail> --out-dir <dir> [--e2e-module <n>]` | `args.rs` + `main.rs` (`run_e2e`) | 🟢 neu in Lane 116 |
| `--e2e --dry-run` (Plan ohne I/O) | `main.rs` (`run_e2e`) | 🟢 neu in Lane 116 |

Was Lane 116 geschlossen hat (additiv, kein fremder Umbau): `mod e2e` war toter
Code (GATE_STATUS-Befund: `dead_code`-Warnungen, `grep e2e::run|tiny_arm ...
main.rs` ohne Treffer). Jetzt: `--e2e`/`--out-dir`/`--e2e-module` in `args.rs`
(neue Felder + Parse-Arme + `--help`-Text, bestehende Flags unverändert),
Dispatch `run_e2e` in `main.rs` (genau 1 Input, Dry-Run-Plan, Report auf stdout,
Exit 0/1/2 via `exit_code_for`). Endgültige Flag-Parität bleibt Lane-38-Hoheit;
die `--e2e`-Schreibweise folgt der Vorgabe aus `lanes/45-e2e-tiny-arm-fails.md`
(„Manuell"-Abschnitt) und ist bis Lane 38 vorläufig.

Manuell (Integrator, nicht diese Lane — unverändert aus Lane 45):

```sh
sail-rs --e2e sail/test/lean/SailTinyArm.sail --out-dir <dir>
# Fail-Liste prüfen, nach Lane-Fixes `lake build` im out-dir
```

## 2. Fail-Ampel (je Fail aus Lane 45)

Legende: 🟢 Treiber-seitig abgedeckt · 🟡 Code liegt, Rig-Bestätigung ausstehend ·
🔴 offen, Owner-Lane QUEUED.

| ID | Stufe | Owner (Status) | Ampel | Beleg / Notiz |
|----|-------|----------------|-------|---------------|
| F1 | includes | lane-38 (QUEUED) | 🔴 | Z. 5, 748–751 (`concurrency_interface*.sail`); 🟢 Treiber-seitig: Pragma-Zensus + `prelude.sail`-Sonde in `run()` |
| F2 | parse-full-grammar | lane-17 (QUEUED) | 🔴 | Z. 599/658/687/703 (`@`), 590/594 (scattered), 757–837 (`instantiation`); `sail_parse::parse_file` verifiziert ( Signatur `(&str,&str)->ParsedFile`, `Def`-Varianten ok) |
| F3 | ast-unify | lane-47 (QUEUED) | 🔴 | Zentraler Blocker: `body_src`-Rohtext überall; blockiert Resolver/Ty/Solver/Effects/Lowering/Mapping |
| F4 | lean-typedefs | lane-30 (QUEUED) | 🔴 | Z. 15–25 (bitfield), 67ff (enum), 80ff (struct), 580–586 (union) |
| F5 | lean-bitvec-exprs | lane-22 (QUEUED) | 🔴 | Z. 616–746 (`^`, `+`, `truncate`, `sail_*_extend`) |
| F6 | lean-int-arith | lane-23 (QUEUED) | 🔴 | Z. 486 (`range(0,31)`), 603–605, 708–710 |
| F7 | lean-patterns | lane-21 (QUEUED) | 🔴 | Z. 742–745 (`match` auf `option(ast)`), 574/579 (Overloads), scattered-Dispatch |
| F8 | lean-effects | lane-28 (QUEUED) | 🔴 | Z. 524–577/616–653 (Register), 745/771–799 (`exit`/`assert`) |
| F9 | lean-concurrency | lane-29 (QUEUED) | 🔴 | Z. 757–837; E1 beachten (Lean bleibt Master, nur Durchreiche) |
| F10 | diagnostics | lane-35 (QUEUED) | 🔴 | Span+Pass+Snippet, Exit-Codes, `--explain` für alle obigen |

Zusatz-Zeilen (Pipeline-Infra, keine Lane-45-Fails):

| Zeile | Owner (Status) | Ampel | Notiz |
|-------|----------------|-------|-------|
| lean-prelude-Stub (`print_prelude`) | lane-10 (Worktree aktiv) | 🟢 | Signatur `(&mut W)->Result` verifiziert, Aufruf in `run()` ok |
| Lake-Skelett (`LakeProject::layout`) | lane-12 (Worktree aktiv) | 🟢 | `new/register/layout()->Vec<GeneratedFile{path,contents}>` verifiziert |
| Register-Zensus (`parse_register_lines/extract_registers/lean_of_sail_typ`) | lane-19 (implizit, sail-regs) | 🟢 | Signaturen verifiziert |
| OoC-Streaming (`stream_rewrite`, `run_with_ooc`) | lane-62 (AKTIV) | 🟡 | Code + `#[cfg(test)]` liegt, Rig-Lauf ausstehend |

## 3. Gegen Main geprüft (gelesen, nichts kopiert/fixiert)

- `sail_parse::parse_file(&str,&str)->ParsedFile{path,defs}` (`parser.rs:548`);
  `Def::{Fundef,Val,Mapdef,Pragma{name,arg:PragmaArg}}` (`mini_ast.rs`) — passt zu
  `fun_names` + Pragma-Zensus in `e2e.rs`.
- `sail_lean::{LeanPrinter::with_default/print_prelude/render_fun_stub, FunSig{name,args,ret,effect,span}, Typ::Unit, Effect::{Stateful,Pure}}`,
  `Span`/`FileId` via `sail_ast`-Re-Export (`sail-lean/src/lib.rs:44`) — passt zu `stub_sig`.
- `sail_lake::{LakeProject::new/registers/layout, RegisterEntry=(String,String), GeneratedFile{path,contents}}` — passt.
- `sail_regs::{parse_register_lines,extract_registers,lean_of_sail_typ}` — passt.
- `sail_prelude::get("prelude.sail")->Option<&PreludeFile>` — passt.
- Keine Kollision: `--out-dir`/`--e2e`/`--e2e-module` sind in keiner
  OCaml-Passthrough-Liste (`args.rs`: `is_bool_passthrough`/`is_value_passthrough`
  ohne Treffer); `-`/`_`-Äquivalenz greift automatisch (`--out_dir` == `--out-dir`).

## 4. Offen / nächste Schritte (mit Owner)

- Alle F1–F10: Owner-Lanes QUEUED → Abbau dort, danach Integrator-Lauf
  (`--e2e` + `lake build` im out-dir, mit Memory-Limits). Kein Lane-116-Follow-up.
- Text-Diskrepanz (Hinweis, kein Fix in dieser Lane): `e2e.rs`-Fail-String F2 nennt
  Lane 17 „aktiv", `lanes/17-grammar-menhir-port.md` sagt QUEUED — gehört Lane 45.
