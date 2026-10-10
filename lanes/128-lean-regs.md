# Lane 128 — lean-regs (Bug-Hunt-Fixes)
Branch: `lane/128-lean-regs` | Status: AKTIV

## Ziel
Lean-Emission HOCH 1–6 + 14: Inhabited-Dedup pro Typ, doc_reg_info vs
doc_instantiations trennen, monadische Frames (SailME.run/Id.run/ExceptM.run),
Prelude-Monaden (PreSailM/PreSailME), monad_of 2D (is_monadic × early_return),
provably_nneg statt Konstanten-Heuristik, read/writeReg-Pfeile.

## Aufgaben
- Gegen OCaml-Referenz (pretty_print_lean.ml Stellen aus Fundliste) angleichen.
  Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Alle Punkte behoben oder begründet abgewiesen (Rig-Beweis offen).
- HOCH 1 (Inhabited-Dedup pro Typ, `sail-regs`): `inhabited_instances`
  deduped nach `lean_typ` (erster Ctor gewinnt), wie OCaml `add_reg_typ`/
  `typ_map` + `sail-lake`-Golden `register-v2.txt`.
- HOCH 2 (doc_reg_info vs doc_instantiations, `sail-regs`):
  `doc_reg_info` = Enum + Abbrev only (OCaml :1499-1501); Inhabited-Block
  neu in `doc_instantiations` (Arch/Monaden bleiben `sail-lake`-Sache).
- HOCH 3 (Frames, `sail-lean`): `FunSig` trägt
  `(is_monadic, early_return, has_loop)`; `function_frame_suffix`
  delegiert an `sail_except::function_frame` (eine Matrix, OCaml :1167-1174).
- HOCH 4 (Prelude, `sail-lean`): `print_prelude[_v1/_v2]` emittiert
  `PreSailM/PreSailME` + `exception`-Abbrev (OCaml `doc_monad_abbrev`,
  :1503-1522) statt IO-Stubs; `sail-lake`-Golden bleibt Master.
- HOCH 5 (monad_of 2D, `sail-lean`): `ret_monad_of` (OCaml :1164),
  `print_ascribed_monad` (`prepend_monad`, :725-730), `throw_name`
  (:935-937); `EarlyReturn` heisst jetzt korrekt `SailM … := SailME.run do`.
- HOCH 6 (provably_nneg, `sail-typedef`): `atom`/`range` nutzen
  `nexp_provably_nneg` über eigene Quantoren (Konstante ≥ 0 oder
  Kid-Bound, inkl. `==`); Solver-Beweise bleiben Lane 05.
- HOCH 14 (Pfeile, `sail-regs`): `read_reg_in`/`write_reg_in` mit
  `as_monadic` — pure Positionen `(← readReg …)`/`(← writeReg …)`
  (OCaml :859/:1044-1046 via `wrap_with_left_arrow`).
- Tests als Code (E3, nicht ausgeführt); `sail-cli`/e2e auf Triple
  umgestellt. `#![forbid(unsafe_code)]` überall unangetastet.
