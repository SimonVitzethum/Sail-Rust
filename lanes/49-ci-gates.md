# Lane 49 — ci-gates
Branch: `lane/49-ci-gates` | Status: QUEUED

## Ziel
E2- + Unsafe- + Budget-Gates in CI (mit Lane 13/14/15 verzahnen).

## Aufgaben
- E2-Gate: `grep -rn sorry --include=*.lean` über Generat muss leer sein.
- Unsafe-Gate: `cargo geiger --forbid-only` + `unsafe`-Grep.
- Budget-Gate: Lane-14-Budgets als CI-Schranke (warnen, nicht blockieren bis stabil).

## Done
- `.github/workflows/ci.yml` erweitert, alle 3 Gates dokumentiert.
