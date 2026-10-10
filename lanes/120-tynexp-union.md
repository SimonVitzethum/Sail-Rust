# Lane 120 — tynexp-union-repair (Welle-7-Fallout)
Branch: `lane/120-tynexp-union` | Status: AKTIV

## Ziel
23 Rig-Errors aus Welle-7-Union (Log limited-run-20261010-200547-116021.log lesen!):
MiniExp::Var-Tupel-vs-Struct (E0769×4, E0559×6), Deref von NExp/Symbol/i128 (E0614×11),
Var-Doppeldefinition (E0428), Mismatch (E0308) — Stellen: ty/nexp.rs, ty/conv.rs,
nexp/expr.rs, advanced.rs, bitvec/conv.rs.

## Aufgaben
- Formen vereinen (105-Tupel-Var + 71-Symbol-Payloads + 108-NExp-Shape), minimal,
  Semantik wahren. Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Alle 23 Fehler weg (Rig-Beweis offen).
