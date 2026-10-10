# Lane 74 — ooc-summaries (E4 für 08/09)
Branch: `lane/74-ooc-summaries` | Status: AKTIV

## Ziel
E4-Summaries für interprozedurale Pässe: Callgraph-/Mono-/Const-Prop-Pässe
(sail-effects/sail-lower/sail-rewrite lesen) liefern Summary-Typen — Bodies bleiben
auf Platte (sail-ooc SummaryTable nutzen, nicht doppeln).

## Aufgaben
- Summary-Typen + Export-Schnittstelle je Pass, Tests als Code (NICHT ausführen, E3).
- `#![forbid(unsafe_code)]`. Fremde Crates nicht umbauen.

## Done
- Summaries definiert + angebunden, Doku für Treiber.
