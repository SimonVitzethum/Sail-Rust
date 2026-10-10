# Lane 35 — error-diagnostics
Branch: `lane/35-diags` | Status: QUEUED

## Ziel
Fehler mit Span statt `failwith`/`unreachable`.

## Aufgaben
- Fehler-Typ mit `Span + Pass + Sail-Source-Snippet`, Exit-Codes, `--explain`-Tabelle für alle heutigen `failwith`-Stellen in `pretty_print_lean.ml`.

## Done
- Jeder Fehler verweist auf Lane-Spec + Workaround.
