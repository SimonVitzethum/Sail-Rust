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
5. **`IndexRange` parens removed (Lane 76, static audit — E3, no build)**:
   `"(" IndexRange ")"` duplicated `AtomicTyp`'s `"(" Typ ")"` for every
   parenthesised type, so `(t)` was ambiguous (two trees, same
   `Single(t)` AST) and `(a, b)` was shift-vs-reduce on `,` — while being
   the only spelling for `(a..b)` / `(a@b)`. Menhir's shift preference
   resolves `(t)`/`(a, b)` to the `Typ` reading, which is exactly what the
   removal keeps; only parenthesised `..`-ranges/`@`-concats are lost
   (bare `a..b` / `a@b` still parse). Corpus: 0 occurrences in `sail-lib`
   (no `bitfield`s at all there; `@` only as expression operator) —
   `tests/corpus.rs` guards this on the Rig. `pub MPat` was demoted to a
   private rule alongside (only used inside `MPExp`; the wiring below
   never listed an `MPatParser`).
6. **`RDefBody` strict (Lane 87, E4 `Block` shape, re-applied on this
    branch)**: the old `(<RIdDef> ",")* <RIdDef>?` list was nullable AND
    used as `RDefBody?` in the `bitfield` arm, so `{}` had two trees
    (Rig: local ambiguity at `sail.lalrpop:1288:5-1291:5`). The list is
    now strict — non-empty, right-recursive, explicit trailing-, arm,
    exactly the validated `Block` shape from the E4 toy — while the use
    site keeps `RDefBody?`, so `{}` still parses (via `None`) and every
    non-empty body parses to the same single tree as before. Accepted
    language unchanged (no corpus impact;
    `tests/grammar_coverage.rs` pins `{}` / trailing-, shapes on
    the Rig).
7. **Nullable-list family strict (Lane 98, same shape)**: `Enum`,
    `ExpList`, `CaseList`, `StructFields` had the same nullable
    `(<X> ",")* <X>?` shape, which LALRPOP expands to `(...)+`/`(...)*`
    intermediates whose shifts/reduces compete on every `X`-leading
    token (Rig `659:5-662:5` `CaseList`, `779:5-782:5` `ExpList`,
    `1215:5-1218:5` `Enum`, `309:5-312:5` local ambiguity via
    `StructFields` `{}`). All four are now strict (non-empty,
    right-recursive, explicit trailing-, arm). Use sites that accepted
    empty bodies in the port keep `?` (`match`/`try` `CaseList?`,
    `Enum?`, `StructFields?` in `struct` records), so `{}` still parses
    via `None` (the anon-`TypeUnion`-record `?` fell away in Lane 104,
    fix 13);
    `ExpList` needed no use-site change (every site already guards it
    with a leading element plus `,` or a separate `"()"` arm).
    Accepted language unchanged.
8. **Optional tails as explicit arms (Lane 98)**: `IndexRange`
    (`@`, Rig `1267:5-1270:5`) and `AtomicIndexRange` (`..`, Rig
    `1274:5-1277:5`) are two arms each instead of `(...)?` tails;
    `LoopExp` `by`/`in` (Rig `610:5-617:5`) is four arms covering all
    combinations; `TypSchm` with/without `effect` (Rig `358:5-364:5`)
    is two arms. The old inline `?` tails left empty productions whose
    reduces competed with the keyword shifts. Accepted language
    unchanged.
9. **Bare `let`/`var` out of `BlockItem` (Lane 98)**: `BlockItem`
    carried bare `let`/`var` without `in` while `PlainExp` carries the
    `in` forms, so after `let p = E` a reduce competed with the `in`
    shift (Rig `682:5-691:5`). The bare forms are now explicit `Block`
    arms carrying their `;` terminator (bare / trailing-`;` /
    `;`-continued, for `let` and `var`); after `let p = E` the
    lookahead decides by shift only (`in` continues the `PlainExp`
    form, `;` continues the block) and only `}` reduces.
    `BlockItem` is just `Exp` now. The missing body still defaults to
    `()`; `{ let p = E }`, `{ let p = E; }`, `{ let p = E; ... }`
    parse exactly as before — accepted language unchanged.
10. **Rest-list family strict (Lane 99, Rig `limited-run-…-175801`)**:
    every remaining nullable `(<X> ",")* <X>?` list is strict now
    (non-empty, right-recursive, explicit trailing-`,` arm, Lane-87
    shape): `TypList` (Rig `310:5-313:5`), `NumList` (Rig
    `324:5-327:5`, headline — the empty-reduce competed with every
    `KOptList`-leading token after `{`), `PatList`, `MPatList` (Rig
    `1083:5-1086:5`), `FPatList`, `FMPatList` (Rig `1143:5-1146:5`),
    `FexpList` (Rig `910:5-913:5`), `VecUpdList` (Rig `935:5-938:5`),
    `MapClList`, `ParamKOptList`, `EnumFns`, `TypeUnions` (Rig
    `1379:5-1382:5`), `SubstList`, `ExternBindList` (split out of the
    inline `ExternBody` braces), `AttrKVList`, `AttrDataList` (split
    out of the inline `AttrData` brackets). Use sites that accepted
    empty bodies in the port keep `?` (`{}` / `{||}` / `[||]` /
    `[]` / `()` via `None`: `NumList`, `PatList`-vectors,
    `MPatList`-vectors, `FPatList`, `FMPatList`, `FexpList`
    incl. `StructUpdate`-`with`, `VecUpdList`-`with`, `MapClList`,
    `ParamKOptList` (`TyArgs` too), `EnumFns`, `TypeUnions`,
    `ExternBindList`, `AttrKVList`, `AttrDataList`), so the accepted
    language is unchanged. Tuple use sites (`Typ`, `Pat`, `MPat`,
    `Exp`) keep three explicit arms (`(x)`, `(x,)`, `(x, …)`) —
    this also restores the `(e,)` single-tuple that strict `ExpList`
    alone would reject. `PatIdTail` / `MPatIdTail` keep their
    separate `"()"` arms, so strictness removes the old `()`
    double tree (same effect as fix 6 for `{}`).
    `tests/grammar_coverage.rs` pins the empty/trailing shapes.
11. **Remaining `(...)+` / `(...)?` intermediates as explicit arms
    (Lane 99)**: the `(<PatOp> <AtomicMPat>)+` mapping-pattern chain
    is a right-recursive `MPatOps` rule; the `("." <RawId>)*`
    `ConfigKey` tail is two right-recursive arms (Rig `1282:5-1286:5`);
    the `CallArgs?` / `IdCallTail?` / `PatIdTail?` / `MPatIdTail?` /
    `MPatSuffix?` use sites are split into bare/tailed arms; the
    `VecUpdTail?` tail is inlined into three `VecUpd` arms (Rig
    `918:5-926:5`); `FMPat`-`=` (Rig `1136:5-1139:5`),
    `UnbracketedTyParam`-`constraint` (Rig `1220:5-1223:5`) and
    `RegDef`-`=` (Rig `1509:5-1513:5`) are two/four explicit arms.
    Same combinations as before; accepted language unchanged
    (except degenerate `instantiation I with` with no substitution,
    which now errors instead of yielding an empty list).
12. **Residuals (already E4, Rig decides)**: the already-strict rules
    still report reduce-vs-shift on continuation-leading tokens
    (`TypSchm`-`effect` 377, `Exp`-`else` 544, `LoopExp`-`by`/`in`
    640/648, `CaseList`-`,`/`-cont` 719/725, `Block`-`;` 745/750/764/
    769, `ExpList`-cont 889, `IndexRange`-`@`/`..` 1395/1404,
    `StructFields`-cont 1363, `RDefBody`-`,`/`-cont` 1423/1429).
    These shapes are unchanged by this lane; they share one signature
    (completed arm vs continuation shift) and are expected to quiet
    down once the macro-chunk states of fixes 10–11 are gone — if the
    Rig still reports them, that is a follow-up with Rig evidence,
    not another blind reshape. Unreported `(...)?` tails (`TypQuant`
    `,`, `PatexpTyp` `->`, guards, `Fexp`/`PlainExp` `=`, …) are
    deliberately untouched (minimal diff).
13. **Anon-`TypeUnion`-record strict (Lane 104)**: `C : {}` reduced
    two ways — `Id(C, NSet([]))` via `AtomicTyp`'s `"{" NumList? "}"`
    and `AnonRecord(C, [])` via `"{" StructFields? "}"` (Rig
    `limited-run-20261010-193612`, local ambiguity at
    `sail.lalrpop:316:5-316:62`; siblings: `TypeUnions`-tail `,`,
    `union`/`newtype` use sites, all one rule). Menhir
    (`parser.mly:1136`) requires non-empty `struct_fields` here, so
    the arm takes the strict list; the empty brace keeps parsing as
    the `NSet` id-arm (port keeps empty `NSet`, fix 10) and the
    `struct`-record use site keeps `?`. Accepted language unchanged;
    `tests/grammar_coverage.rs` pins the single tree.

6. **`RDefBody` strict (Lane 87, E4 `Block` shape)**: the old
    `(<RIdDef> ",")* <RIdDef>?` list was nullable AND used as `RDefBody?`
    in the `bitfield` arm, so `{}` had two trees (Rig: local ambiguity
    at `sail.lalrpop:1288:5-1291:5`). The list is now strict — non-empty,
    right-recursive, explicit trailing-, arm, exactly the validated
    `Block` shape from the E4 toy — while the use site keeps `RDefBody?`,
    so `{}` still parses (via `None`) and every non-empty body parses to
    the same single tree as before. Accepted language unchanged (no
    corpus impact; `tests/grammar_coverage.rs` pins `{}` / trailing-,
    shapes on the Rig).

## D1/D2 — unchanged, evaluated per corpus

- **D1** (`AtomicIndexRange`: `Typ (".." Typ)?` competing with tuple `,`
  / `..` inside `name : range`) is subsumed by fix 5 above: the only real
  ambiguity was the `"(" IndexRange ")"` wrapper, not the `Typ` rules —
  those stay UNCHANGED. Whether any residual D1 shape surfaces as a real
  LALR conflict is decided by the first Rig build.
- **D2** (`AtomicTyp` parenthesised singleton vs tuple) is UNCHANGED and
  was never ambiguous on its own (`","` vs `")"` after `"(" Typ` are
  disjoint); it stays pending the Rig build all the same.
- Should the Rig build still report a conflict here, the candidates are a
  comma-free `Typ` subset inside `IndexRange` (D1) and an explicit
  paren/tuple split (D2), at the cost of rejecting currently accepted
  inputs — hence no blind fix here.

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
- Parenthesised index ranges (`(a..b)`, `(a@b)`) are rejected (wrapper
  removal, fix 5); bare `a..b` / `a@b` still parse. No occurrences in
  `sail-lib` (no `bitfield`s there at all).

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
- `build.rs`: `lalrpop::process_root()`, with a clear `eprintln!` + exit 1
  on failure (no `panic!`/`unwrap`/`expect`).
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
