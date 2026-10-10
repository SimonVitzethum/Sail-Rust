# Lane 66 — corpus-eval (D1/D2)
Branch: `lane/66-corpus-eval` | Status: AKTIV

## Ziel
Lane-59-D1/D2-Abweichungen per Korpus bewerten (rein analytisch, E3: keine Builds).

## Aufgaben
- Sail-Korpus (sail/lib, test-Modelle) lesen: Vorkommen von nacktem `if` als Then-/Else-Branch,
  `else`-`if`-Ketten ohne Abschluss-`else`, bare `let`/`var`, `{}` zählen + als Fixture-Liste
  (CORPUS.md) ablegen. Kein Parser-Bau, keine Läufe.

## Done
- CORPUS.md mit Zählungen + Empfehlung je Abweichung.
