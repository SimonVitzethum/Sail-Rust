# REGS_NEEDED.md — what `sail-regs` needs from the frontend lanes

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
