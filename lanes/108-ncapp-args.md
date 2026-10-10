# Lane 108 — ncapp-args (Typ-/Bool-Args)
Branch: `lane/108-ncapp-args` | Status: AKTIV

## Ziel
Reviewer-5-MEDIUM-HIGH: NC_app + NC_equal/not_equal über typ_arg (Typ/Bool)
(OCaml Ast.ml:152-163) — NExp-only ist zu eng für ARM-Constraints.

## Aufgaben
- Typen + Konverter + eval/simplify-Anbindung, Tests als Code (NICHT ausführen, E3).
  `#![forbid(unsafe_code)]`.

## Done
- Typ-/Bool-Args darstellbar.
- Lane 108 (E3, ungebaut): `TypArg::{NExp,Typ,Bool}` + `NTyp`/`NQuant`/`NKind`
  in `sail-nexp::expr`; `Equal`/`NotEqual`/`App` ueber `typ_arg`-Sorten;
  neues `conv`-Modul (`*_from_ast`/`*_to_ast` total bzw. fallibel);
  `eval` (`typarg_eq`: gleiche Args `true`, gemischte Sorten `false`,
  ungleiche Typen `None`), `simplify`/`normalize` (`eq_join` je Sorte,
  `simplify_typarg`/`normalize_typarg`, kanonische `App`-/`Equal`-Args),
  `Facts::from_constraint` nur fuer `(NExp,NExp)`-Paare; Tests als Code in
  `crates/sail-nexp/tests/ncapp_args.rs`; Solver-Fallout
  (`query::typarg_key`/`ntyp_key`, `z3_backend::typarg_eq_bool`)
  beweisgesund (exakt wo moeglich, sonst frisch); Operator-Tabelle
  nachgezogen.
