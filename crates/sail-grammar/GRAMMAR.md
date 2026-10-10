# GRAMMAR.md — Menhir (`parser.mly`) → LALRPOP port (`sail-grammar`)

Status: **skeleton**. The crate ships the hand-written lexer
(`src/lexer.rs`), the skeletal AST (`src/ast.rs`), the LALRPOP grammar
(`src/sail.lalrpop`) as a **documentation artifact**, and rule-coverage
tests (`tests/grammar_coverage.rs`). The LALRPOP tables are deliberately
_not_ generated: `build.rs` is a no-op and the crate has no `lalrpop`
dependency. The `parse_*` entry points lex their input and return
`SKELETON_MSG` until the conflicts below are resolved.

## Sources

| File | Size | Role |
| --- | --- | --- |
| `sail/src/lib/parser.mly` | 1484 lines, ~100 named rules | ported (Exp/Pat/Typ/Def) |
| `sail/src/lib/lexer.mll` | token definitions | mirrored by `lexer.rs` |
| `sail/src/lib/infix_parser.mly` | 468 lines | **out of scope** (precedence climbing stays in elaboration) |
| `sail/src/lib/project_parser.mly` | 192 lines | **out of scope** (project files, not Sail source) |

Port files: `src/sail.lalrpop` (1433 lines, 124 rules), `src/ast.rs`
(526 lines), `src/lexer.rs` (~850 lines).

## Why skeleton: LALR(1) conflicts

`cargo build` with `lalrpop::process_root()` rejects the direct port with
shift/reduce conflicts. Menhir tolerates these (resolves to shift);
LALRPOP does not. Observed conflicts (rule refs to `src/sail.lalrpop`):

1. **Dangling `else`** (`Exp`, ~line 493):
   `Exp = IfHead (*) | IfHead (*) "else" Exp`. The factored
   `if c then t` head with optional `else` still conflicts on `else`
   (innermost binding). Menhir shifts; LALRPOP reports reduce-vs-shift.
2. **`in` after `foreach … by` step** (`LoopExp`, ~line 563):
   `LoopExp = Ident "from" Exp ToOrd Exp "by" Exp (*) "in" Typ`
   conflicts with `Exp`'s `let … in` / `var … in`: on lookahead `in` the
   parser can reduce `LoopExp` or shift `in` into the step expression.
   Same root cause as Menhir's documented `loop_exp` shift preference.
3. **Index ranges vs tuples** (`AtomicIndexRange`, ~line 1202):
   `AtomicIndexRange = Typ (*) | Typ (*) ".." Typ` conflicts on both
   `".."` and `","`, because `Typ` itself contains tuple
   `"(" Typ "," … ")"` (`AtomicTyp`). A `Typ` followed by `,`/`..`
   inside `name : range` is ambiguous between tuple-type and range.
4. **Parenthesised vs tuple types** (same area): `AtomicTyp =
   "(" Typ (*) ")" | "(" Typ (*) "," TypList ")"` — the classic
   parenthesised-singleton vs tuple conflict, shifted in Menhir.

Conventions already applied in the port (insufficient for the above):
every Menhir `A | A B` common-prefix rule is factored to `A B?` with
LALRPOP `?`/`*`; semantic actions are AST constructors only.

## Dropped OCaml action logic (intentional)

- Same-operator checks for pattern/type infix chains (`same_pat_ops`
  style errors) — chains are stored raw in `InfixChain`, folding happens
  later.
- Deprecation warnings/paths (e.g. `MPExp "=>" Exp` kept as
  `MapCl::ForwardsDeprecated` without warning).
- Name mangling, `fix_extern`, config-key handling beyond key storage.
- `defs_post_trailing_constraint_list` constraint reordering — `Defs`
  is a plain `Def*` sequence.
- Source locations (byte offsets live in the lexer only).
- Custom `error` productions / parser diagnostics.

## Coverage: `parser.mly` → port

Exp rules (`exp`, `atomic_exp`, `case`, `block`, `loop_exp`,
`fexp_exp`, `vector_update`, `exp_list` …) → `Exp` + `ExpSuffix` +
`FieldExp` + `VectorUpdate` + `PatExp` + `LoopHead`; all 33 `Exp`
constructors built in tests.

Pat rules (`pat`, `atomic_pat`, `fpat`, `lit`, `negative_lit`,
`pat_op`, `mpat`, `atomic_mpat`, `fmpat`, `mpexp`) → `Pat` (14
variants) + `MPat` + `MPExp`; `as`/`match` annotation keywords unified
as `AsKw`.

Typ rules (`typ`, `infix_typ`, `typ_no_caret`, `atomic_typ`, `tyarg`,
`kind`, `kopt`, `typquant`, `typschm`, `effect_set`, `num_list` …) →
`Typ` (13 variants) + `Kind` (5) + `KOpt` + `TypQuant` + `TypSchm`;
register-type `register(t)` kept as `Typ::App("register", …)`;
`dec`/`inc`/`2^`/prefix ops kept as names (`negate`, `__deref`,
`pow2`).

Def rules (`def_aux`, `def`, `fun_def`, `funcl`, `funcl_patexp_typ`,
`map_def`, `mapcl`, `type_def`, `type_union`, `struct_field`,
`enum_member`, `index_range`, `r_id_def`, `val_spec_def`,
`register_def`, `outcome_spec_def`, `scattered_def`, `subst`,
`overload_def`, `loop_measure(s)`, `typaram`, `def_eof`, `file` …) →
`Def`/`DefAux` (22 `DefAux` variants) + `FunDef`/`Funcl` +
`MapDef`/`MapCl` + `TypeDef` + `ValSpec`/`Extern` + `RegDef` +
`ScatteredDef` + `Pragma`; `mutual { … }` → `DefAux::MutRec`.

Lexer: full `%token` set mirrored as unit `Tok` variants with payloads
only for `Id`/`TyVar`/`Num`/`Bin`/`Hex`/`Real`/`Str`/`OpId`/`DocBlock`/
`DocLine`/`Pragma`/`StructPragma`/`Attr`/`Fixity`; comments skipped,
`///` + `/*! … */` kept as doc tokens, `$[…]` / `$pragma` forms kept.

## Next steps (to generate tables)

- Give `else` an explicit precedence (or split `Exp` into if/non-if
  levels with `%precedence`-style layering) instead of the factored
  optional-`else`.
- Split the `Typ` used inside `IndexRange` into a comma-free subset so
  ranges no longer compete with tuples.
- Disambiguate `LoopExp … "by" Exp` vs `in` by requiring the order
  annotation or delimiting the step expression.
- Re-enable `lalrpop` build-dep + `lalrpop_mod!(sail)` in `lib.rs`
  once `cargo build -p sail-grammar` is conflict-free, and turn the
  skeleton `Err` tests into real parse tests.
