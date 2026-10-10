# REGS_NEEDED.md — what `sail-regs` needs from the frontend lanes

> Lane 61 status (MIGRATION.md #26/#27): **BRIDGED**.
> `MinimalDef`/`SailTyp` retained: lowering onto
> `DecSpec{typ: Typ, init: Option<Exp>}` needs the Lane-10 `TypKind`
> pretty-printer plus a real expression grammar for initializers, and
> `sail-cli` consumes this input API. Follow-up owned by Lane 10/02,
> seam wired (`sail-ast` dependency).

`crates/sail-regs` is standalone: it takes [`MinimalDef`] (see
`src/lib.rs`) instead of depending on another worktree's crate.
The mapping to real frontend types is 1:1:

| `sail-regs` input | Lane 01 (`sail-ast`) source | Notes |
|---|---|---|
| `MinimalDef::Register { name, typ, init }` | `DefKind::Register(DecSpec)` (`crates/sail-ast/src/ast.rs`) | `name` = `DecSpec.name` symbol text, `typ` = `DecSpec.typ` rendered to Sail source, `init` = `DecSpec.init: Option<Exp>` rendered to Sail source (`None` = no `= …`) |
| `SailTyp` rendering | `TypKind` pretty-printer (Lane 10 Lean printer stream) | only the spelling matters, e.g. `bits(64)`; `lean_of_sail_typ` maps the common cases |
| `has_init` / `init_sail` | `State.find_registers` third tuple element (`Option.is_some opt_exp`) + `reset.sail` initializers | `initial_values()` prefers the explicit initializer, else `default_init_sail` (mirrors `lookup_init_val` in `sail/src/lib/state.ml`) |

When the real AST is wired up, conversion is:

```rust
// DecSpec { typ: Typ, name: Name, init: Option<Exp>, .. }
//   -> MinimalDef::Register {
//        name: interner.resolve(name.symbol).to_owned(),
//        typ: SailTyp::new(render_typ(&dec.typ)),
//        init: dec.init.as_ref().map(render_exp),
//      }
```

No other frontend data is needed: synonym expansion (`Env.expand_synonyms`)
and bitfield base types (`regval_base_typ`) stay the typechecker's job;
callers pass the already-expanded spelling.

## Lane-64-Bridge (#26/#27, `src/bridge.rs`)

`extract_registers_ast(&[sail_ast::Def], &impl Interner)` implements the mapping
above over real AST defs; `dec_spec_to_minimal` is the 1:1 `DecSpec` conversion.
`render_sail_typ`/`try_render_sail_exp` produce the Sail spellings, and
`lean_typ_of_ast` maps onto the Lane-10 printer types (`sail_lean::{Typ,Nexp}`,
printer used read-only via `LeanPrinter::print_typ`, never modified) with
spelling fallback outside the printer subset. `sail-lean` itself is untouched.
