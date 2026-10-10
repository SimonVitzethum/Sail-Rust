# `RESOLVE_GAPS.md` — Lücken-Doku je Def-Art (Lane 78)

> Stand: Lane 78 löst **alle** [`DefKind`]-Varianten des echten `sail-ast`
> auf (Bridge-Vertrag aus Lane 63 gelesen:
> `crates/sail-parse/src/bridge.rs`, `AST_NEEDED.md` „Bridge-Stand").
> Keine `Wild`/`Unknown`-Reste ohne Owner: jede Def-Art landet in einer
> Tabelle, einer Zählliste oder einer verlinkten Pending-Prüfung.
> Was darunter als **Lücke** steht, ist bewusst an die Owner-Lane
> delegiert (kein Overlap), nicht übersehen.

[`DefKind`]: ../sail-ast/src/ast.rs

## Legende

- **voll**: Name + Namespace + Duplikat- und Linkage-Prüfung implementiert.
- **teil**: Namen/Skelett aufgelöst, Inhalte folgen mit Owner-Lane.
- **Zählung**: kein Namespace (gezählt/gesammelt, kein `distincto`).

## Top-Level-Def-Arten (`DefKind`)

| # | Def-Art | Status | Scope / Prüfung | Lücke → Owner |
|---|---|---|---|---|
| 1 | `Type` (Abbrev/Record/Variant/Enum/Abstract/Bitfield) | voll | Typname → `types`; Enum-Member + Variant-Ctors → `ctors` (`DuplicateCtor`), Kollision → Warnung + `compute_ctor_renames` | Feld-/Payload-Typen ungeprüft (nur Namen) → Typchecker-Lane |
| 2 | `Constraint` | Zählung | Span in `env.constraints`, keine Prüfung | Inhaltliche Constraint-Prüfung → Solver-Lane |
| 3 | `Fundef` | voll | pro Name → `functions` (`DuplicateFunction`, `is_rec`, Klauselzahl); jede Klausel-Arm-Pattern bindet in frischem Scope (`DuplicateValue` bei Doppelbindung) | `and`-Ketten-Gruppierung, Guards/Bodies (`Exp`) → Lane 02 |
| 4 | `Mapdef` | voll | Name → `mappings` (`DuplicateMapping`); jede `Mapcl`-Seite bindet Pattern in frischem Scope | Klausel-Bodies (Bridge: leere Listen) → Lane 02/20 |
| 5 | `Impl` | voll | in `env.impls` + Pass-2-Linkage gegen `scattered_functions` (`DanglingImpl`) | — |
| 6 | `Let` | voll | Pattern-Namen → `values` (`DuplicateValue`) + globaler Scope-Spiegel | `Exp`-Init/Bodies → Lane 02 |
| 7 | `Val` | voll | Name → `valspecs` (`DuplicateValSpec`), `has_extern`-Flag | `Typschm`-Inhalt, Extern-Bindings → Lane 02 |
| 8 | `Outcome` | voll | Name → `outcomes` (`DuplicateOutcome`) | Outcome-Semantik → Solver/CLI-Lane |
| 9 | `Instantiation` | voll | Name → `instantiations` (`DuplicateInstantiation`) | `Subst`-Ziele ungeprüft → Typchecker-Lane |
| 10 | `Fixity` | voll | Name → `fixities` (`DuplicateFixity`), Level als String | — |
| 11 | `Overload` | voll | Name → `overloads` (`DuplicateOverload`); Pass 2: jedes Target muss `val` oder Funktion sein (`UnknownOverloadTarget`) | Überladungs-Auflösung an Aufrufstellen → Typchecker-Lane |
| 12 | `Default` | voll | genau ein `env.default` (`DuplicateDefault`) | — |
| 13 | `Scattered` | voll | s. Untertabelle; Klauseln/Ctors inkl. Rename-Rechnung | dateiübergreifende Gruppierung → Lane 11 |
| 14 | `Measure` | voll | Funktionsname → `measures` (`DuplicateMeasure`); Pattern in frischem Scope | Measure-Semantik → Termination-Lane |
| 15 | `LoopMeasures` | voll | Funktionsname → `measures` (`DuplicateMeasure`) | s. `Measure` |
| 16 | `Register` | voll | Name → `registers` (`DuplicateRegister`) | Init-`Exp`, Semantik → Lane 19 |
| 17 | `InternalMutrec` | voll | jede Gruppe wie `Fundef` (Namen → `functions`), Gruppen in `mutrec_groups` gezählt | Aufteilung in Einzel-`Fundef`s → Lower-Lane |
| 18 | `Pragma` | voll | `env.pragmas` in Quellreihenfolge (repeatable, inkl. `file_start`/`file_end`) | Pragma-Semantik je Key → jeweilige Lane |

## `ScatteredDef`-Unterarten

| Unterart | Namespace/Tabelle | Linkage (Pass 2) |
|---|---|---|
| `Function` / `Mapping` / `Variant` / `Enum` | `scattered_functions` / `scattered_mappings` / `scattered_variants` / `scattered_enums` (`DuplicateScattered`) | `End` setzt `closed=true` |
| `Funcl` | Klausel-Pattern in frischem Scope | `DanglingScatteredClause` ohne offenes `Function` |
| `Mapcl` | beide `Mapcl`-Seiten in frischem Scope | `DanglingScatteredClause` ohne offenes `Mapping` |
| `Unioncl` | Ctor → `ctors` (+ `seen_ctors` für Renames) | `DanglingScatteredClause` ohne offenes `Variant` |
| `Enumcl` | Member → `ctors` (+ `seen_ctors` für Renames) | `DanglingScatteredClause` ohne offenes `Enum` |
| `End` | — | `DanglingScatteredEnd` ohne offenen Kopf (offene Köpfe ohne `End` sind OK: Shard-Split, Lane 11) |

## Bewusste Nicht-Lücken (frozen contracts)

- `compute_ctor_renames`-Signatur/Determinismus (`_ctor`/`_ctor{N}`) — frozen mit Lane 10.
- Namespaces bleiben getrennt: gleicher Name in verschiedenen Tabellen kollidiert nie.
- Top-Level bleibt reihenfolgeunabhängig (Bind-Pass, dann Check-Pass).
- Ctor-vs-Typ-Kollision ist Warnung + Rename, nie Fehler.
