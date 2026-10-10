# Lane 87 — grammar-1288 (Rig-Loop)
Branch: `lane/87-grammar-1288` | Status: AKTIV

## Ziel
Echten LALRPOP-Konflikt `sail.lalrpop:1288:5-1291:5` lösen (Rig-Beweis: process_root läuft).

## Aufgaben
- Konfliktstelle analysieren (E3c/E4-Muster aus GRAMMAR.md), Fix als Code.
  KEINE Builds (E3) — Verifikation läuft über Haupt-Session-Rig, bei Fehlschlag
  kommt Revision per Follow-up. `#![forbid(unsafe_code)]`.

## Done
- Konflikt behoben (Rig-Beweis offen).
- Ursache: `RDefBody` (nullable `(...)* ...?`-Liste) wurde als `RDefBody?`
  verwendet — `{}` hatte zwei Baeume (Rig: Local ambiguity 1288:5-1291:5).
- Fix: strikt (nicht-leer, rechts-rekursiv, Trailing-`,`-Arm) im
  E4-`Block`-Muster; Use-Site behaelt `?`. Sprache unveraendert.
- Hinweis: Rig-Log zeigt weitere Stellen mit demselben bzw. verwandtem
  Muster (Follow-up-Lanes).
