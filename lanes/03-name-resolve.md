# Lane 03 — name-resolve
Branch: `lane/03-name-resolve` | Worktree: `sail-rs/worktrees/lane-03` (nach Merge löschen)

## Ziel
Scopes, Module, Überladungen auflösen; Union-Konstruktor-Kollisionen erkennen (Basis für Ctor-Renames im Lean-Print).

## Aufgaben
- `crates/sail-resolve`: Env-Tabellen (ValSpecs, Typen, Register, Mappings), ` distinto`-Fehler.
- Liefert `compute_ctor_renames`-Äquivalent als reine Funktion.

## Input/Output
- Input: `sail-ast`. Output: resolved AST + Symboltabellen.

## Abgrenzung
- Keine Constraints, kein Z3.

## Done
- Kollisionstests (Typ == Ctor-Name) + `test/lean`-Subset grün.
