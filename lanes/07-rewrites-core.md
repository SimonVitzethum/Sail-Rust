# Lane 07 — rewrites-core
Branch: `lane/07-rewrites-core` | Worktree: `sail-rs/worktrees/lane-07` (nach Merge löschen)

## Ziel
Lean-Rewrite-Pipeline als einzelne, parallele Pässe (Äquivalent zu `lean_rewrites` in `sail_plugin_lean.ml`).

## Aufgaben
- `crates/sail-rewrite`: Pass-Trait (`name`, `run(shard) -> shard`), Pässe: `move_termination_measures`, `realize_mappings`, `remove_vector_subrange_pats`, `undefined`, `tuple_assignments`, `simple_assignments`, `merge_function_clauses`, `early_return`, `rewrite_loops_with_escape_effect`, `top_sort_defs`, `bit_lists_to_lits` u.a.
- Jeder Pass shard-lokal + parallel via `rayon`; keine globalen Mutables.

## Done
- Jeder Pass einzeln an `test/lean`-Beispielen getestet, Reihenfolge konfigurierbar.
