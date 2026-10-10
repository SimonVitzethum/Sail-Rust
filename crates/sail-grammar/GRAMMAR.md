# GRAMMAR.md — Menhir (`parser.mly`) → LALRPOP port (`sail-grammar`)

Status: **generating**. The crate ships the hand-written lexer
(`src/lexer.rs`), the skeletal AST (`src/ast.rs`), and the LALRPOP grammar
(`src/sail.lalrpop`), wired up via `build.rs`
(`lalrpop::process_root()`, `lalrpop` 0.22 build-dep) +
`lalrpop_mod!(sail)` in `src/lib.rs`. The `parse_*` entry points tokenize
their input and run the generated `*Parser`s. Rule coverage:
`tests/grammar_coverage.rs`; Sail-library corpus: `tests/corpus.rs`
(written in this lane, executed later on the Test-Rig — E3, no local runs
here).

## Sources

| File | Size | Role |
| --- | --- | --- |
| `sail/src/lib/parser.mly` | 1484 lines, ~100 named rules | ported (Exp/Pat/Typ/Def) |
| `sail/src/lib/lexer.mll` | token definitions | mirrored by `lexer.rs` |
| `sail/src/lib/infix_parser.mly` | 468 lines | **out of scope** (precedence climbing stays in elaboration) |
| `sail/src/lib/project_parser.mly` | 192 lines | **out of scope** (project files, not Sail source) |

Port files: `src/sail.lalrpop`, `src/ast.rs` (526 lines), `src/lexer.rs`
(~850 lines).

## Conflict fixes (E3c + E4)

The direct port failed with 4 LALR conflicts (Menhir resolves them as
shift; LALRPOP rejects them). The fixes below are validated designs from
scratch experiments with LALRPOP 0.22 (`/tmp/opencode/laltest`, exit 0),
lifted onto the Sail rules in this lane:

1. **E3c — dangling `else`**: open `Exp` vs closed `ClosedExp` WITHOUT an
   `Exp ::= ClosedExp` unit production (that shape was the ambiguous E3b).
   `then`/`else` branches are always `ClosedExp`, and every tail position
   whose value can end the enclosing expression is `ClosedExp` in both
   rules (assignment RHS, `let`/`var` bodies, `return`/`throw`, attribute
   bodies, `do` bodies, `repeat..until` RHS, `internal_*` bodies — any of
   them can sit under a `then` branch). All other positions (conditions,
   scrutinees, arguments, elements, bodies inside braces, top-level tails)
   stay `Exp`. Both rules share the non-`if` forms via `PlainExp`, so
   `else` never lands in `FOLLOW(Exp)` and `if E then C` followed by `else`
   shifts unambiguously (innermost binding). A bare `if..then` without
   `else` still parses in `Exp` positions and defaults to `()`.
2&3. **E4 — dangling `in` (2×, self-inflicted by optional `in`)**:
   expression-level `let`/`var` REQUIRE `in`; bare `let`/`var` without
   `in` exist only as `BlockItem` inside strict `Block`s (non-empty,
   right-recursive, trailing `;` ok). The `in` vs `;`/`}` lookahead keeps
   `let p = E in C` (shift) and bare `let p = E` (reduce) apart. This also
   settles `LoopExp ... "by" Exp "in" Typ`: a step can no longer end in a
   bare `let`/`var` waiting for `in`.
4. **Nullable Block + `with`**: `Block` is strict now (see E4), so after
   `{ Exp` the lookahead `with` can only shift the `StructUpdate` arm of
   `BlockOrUpd` (`;`/`}` reduce the block item instead).

## D1/D2 — unchanged, evaluated per corpus

- **D1** (`AtomicIndexRange`: `Typ (".." Typ)?` competing with tuple `,`
  / `..` inside `name : range`) and **D2** (`AtomicTyp` parenthesised
  singleton vs tuple) are intentionally UNCHANGED in this lane. Whether
  they surface as real LALR conflicts (first Rig build) and whether the
  ambiguous inputs occur in real code (`tests/corpus.rs`) decides the fix:
  candidates are a comma-free `Typ` subset inside `IndexRange` (D1) and an
  explicit paren/tuple split (D2), at the cost of rejecting currently
  accepted inputs — hence no blind fix here.

## Known acceptance deltas vs Menhir (watched by `tests/corpus.rs`)

E3c/E4 trade Menhir's shift-preference for rejections; the corpus test
quantifies them on `crates/sail-prelude/sail-lib`:

- An open `if..then` (no `else`) is rejected in `then`/`else` branches and
  in the `PlainExp` tails (e.g. `let x = 1 in if a then b`,
  `return if a then b`). Menhir binds a trailing `else` innermost and
  accepts; the port requires the inner `if` to carry `else`.
- `else`-`if` chains ending in a bare `if..then` are rejected for the same
  reason (the final `else` branch must be closed).
- Bare `let`/`var` without `in` are rejected outside blocks (call
  arguments, `do` bodies, ...); only `BlockItem`s admit them.
- `{}` is rejected (strict `Block`); no occurrences in the corpus at the
  time of writing (checked via `grep`, non-binding — the Rig decides).

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
`fexp_exp`, `vector_update`, `exp_list` …) → `Exp` + `ClosedExp` +
`PlainExp` + `BlockItem` + `ExpSuffix` + `FieldExp` + `VectorUpdate` +
`PatExp` + `LoopHead`; all 33 `Exp` constructors built in tests.

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

## Wiring

- `Cargo.toml`: `lalrpop-util = "0.22"` dependency,
  `lalrpop = "0.22"` build-dependency.
- `build.rs`: `lalrpop::process_root().unwrap()`.
- `src/lib.rs`: `lalrpop_mod!(sail)` + `parse_*` running
  `FileParser` / `TypParser` / `TypSchmParser` / `ExpParser` /
  `PatParser` / `DefParser` over `lexer::tokenize` output; lex errors
  surface as strings, parse errors as `parse error: {Debug}`.

## Verification (Test-Rig only, E3 — NOT run in this lane)

- `cargo test -p sail-grammar` on the Rig (see `docs/TEST-RIG.md`,
  `scripts/limited-run.sh`): green, incl. `tests/corpus.rs`, and no
  LALR conflicts from `build.rs`.
- If the Rig build still reports D1/D2 conflicts, fix per the D1/D2
  section above; if the corpus reports E3c/E4 deviations in real Sail
  libs, quantify there before touching the grammar.
