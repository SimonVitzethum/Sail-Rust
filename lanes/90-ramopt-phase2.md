# Lane 90 — ramopt-phase2 (Owned-Rewrites)
Branch: `lane/90-ramopt-phase2` | Status: AKTIV

## Ziel
Lane-68-Abgrenzung: `simplify`/`internal_lets` Owned-Rewrites auf subst_cow-Mechanismus
umstellen + restliche Clone-Hotspots aus RAM_ANALYSIS.md (außer Payloads — Lane 71).

## Aufgaben
- Umbau verhaltens-identisch, Tests als Code (NICHT ausführen, E3).
- `#![forbid(unsafe_code)]`.

## Done
- Owned-Rewrites weg, Verifikation via Rig.
