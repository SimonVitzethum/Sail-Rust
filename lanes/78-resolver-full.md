# Lane 78 — resolver-full (Frontend vollständig 3/7)
Branch: `lane/78-resolver-full` | Status: AKTIV

## Ziel
Vollständiger Resolver auf echtem AST: alle Def-Arten (Val/Fundef/Mapdef/Scattered/
Register/Impl/Overload/Pragmas), Scopes, Überladungen, Ctor-Renames.

## Aufgaben
- sail-resolve vervollständigen (Bridge aus Lane 63 lesen), Lücken-Doku je Def-Art.
  Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Alle Def-Arten aufgelöst, keine Wild/Unknown-Reste ohne Owner.
