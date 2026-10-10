# Lane 75 — arm-gap (Lean-Subset-Lücken im ARM-Modell)
Branch: `lane/75-arm-gap` | Status: AKTIV

## Ziel
Welche Konstrukte in arm-v9.4-Quellen liegen außerhalb unseres Lean-Subsets?
 sail-lex/sail-parse (Branch-Stand, nur lesen) zum Scannen nutzen, kein Build.

## Aufgaben
- Scanner-Skizze + GAP.md (Konstrukt, Häufigkeit, Beispiel-Span, Owner-Lane für Erweiterung).
- E3: KEINE Builds/Läufe. `#![forbid(unsafe_code)]` wo Code entsteht.

## Done
- GAP.md vollständig, jede Lücke mit Owner.
