# `RESOLVE_NEEDED.md` — what `sail-resolve` needs from `sail-ast` (Lane 01)

> Lane 61 status (MIGRATION.md #8–#16): **CLOSED**.
> `src/mini_ast.rs` deleted; `Resolver`/`resolve` take `sail_ast::Ast`
> + `&impl Interner`; spans are `sail_ast::Span` (#8). Semantics
> contract unchanged (namespaces, order-independence, ctor warnings,
> `compute_ctor_renames` frozen).

`sail-ast` has not landed yet, so `sail-resolve` currently resolves the
local stand-ins in `src/mini_ast.rs`. This file pins the exact mapping
so Lane 01 (or a later porting PR) can swap the stand-ins for the real
AST without changing resolution semantics.

## Required real-AST surface

| Need | Mini stand-in today | Expected real node (`sail-ast`) | Notes |
|---|---|---|---|
| Program | `mini_ast::Ast { defs }` | `Ast` / definition list | Flat top-level list, order-independent |
| Type definition | `TypeDef { name, kind, span }` | `TD_*` family (`ast.ml`: `type_def`) | Must expose name + kind |
| Struct | `TypeDefKind::Struct` | `TD_struct` | No ctors |
| Enum + members | `TypeDefKind::Enum(Vec<CtorDecl>)` | `TD_enum` + `E_ctor` list | Member names + spans |
| Union + variants | `TypeDefKind::Union(Vec<CtorDecl>)` | `TD_variant` + `Tu_*` list | Member names + payload types (payloads out of scope for Lane 03) |
| Abbrev / alias | `TypeDefKind::Abbrev` | `TD_abbrev` / `TD_alias` / `TD_bitfield` | Name only; bitfield details → Lane 30 |
| Fun signature | `ValSpec { name, span }` | `VS_val_spec` (`val_spec`) | Name (+ extern info later) |
| Register | `RegisterDef { name, span }` | `DEC_reg` | Name only; semantics → Lane 19 |
| Mapping | `MappingDef { name, span }` | `DEC_mapping` / `MAP_*` | Name only; encode/decode → Lane 20 |
| Other top-level | `Def::Other { name, span }` | `DEF_fundef`, `DEF_scattered`, `DEF_pragma`, … | Function bodies/overloads → Lane 08; must at least yield a name |
| Location | `Span { line, col }` | `l` / `Parse_ast` loc | Line/col suffices for diagnostics today |

## Semantics contract (must hold after the swap)

1. **Namespaces stay separate**: a type `f` and a val-spec `f` do not
   collide. Only same-kind duplicates are `distincto` errors.
2. **Top level is order-independent**: two passes (bind, then check).
   No declaration-before-use requirement at top level.
3. **Ctor-vs-type collision is non-fatal**: it produces a warning plus a
   `compute_ctor_renames` entry, never a hard error. Downstream (Lean
   printer, Lane 10) applies the rename map.
4. **Duplicate ctor across owners is a hard error** (`DuplicateCtor`)
   *and* still yields a rename entry so printing can proceed.
5. **`compute_ctor_renames` signature is frozen**: purity,
   determinism (sorted iteration), `_ctor`/`_ctor{N}` scheme. Do not
   change its output without coordinating with Lane 10.

## Swap checklist (Lane 61: done except JSON roundtrip)

- [x] Replace `mini_ast::*` imports with `sail_ast::*` (add
      `sail-ast = { path = "../sail-ast" }` dependency).
- [x] Delete `src/mini_ast.rs`, keep this file updated (mark done).
- [ ] Add a JSON-roundtrip test if `sail-ast` offers one (Lane 01 Done).
      Deferred: needs `serde` derives on `Env`/`ResolvedProgram`
      (follow-up, serde already available via `sail-ast`).
- [x] Keep `#![forbid(unsafe_code)]` and `cargo test -p sail-resolve` green.
