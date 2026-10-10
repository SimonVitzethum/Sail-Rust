# Lane 70 — main-hygiene (Reviewer-3/4 + Audit-Follow-ups)
Branch: `lane/70-main-hygiene` | Status: AKTIV

## Ziel
Offene Befunde schließen (jeweils klein, präzise):
1. Reviewer-4 mittel (sail-ooc): `HotToken`-Drop ohne `seal` muss Barriere freigeben
   (Drop-Impl, Doku lügt sonst) — `barrier.rs:83-89`.
2. Reviewer-4 mittel (sail-ooc): `get`/`prefetch` müssen `max_live_bytes` prüfen wie `spill`
   (Budget sonst umgehbar) — `warm.rs:184-199` vs `:151-164`.
3. Audit: `z3` optional-Dep 0.12 → ≥0.21 (RUSTSEC-2026-0295); API-Brüche anpassen.
4. Audit: `crossbeam-channel` Requirement ≥0.5.15 (CVE-2025-4574-Bereich).
5. Reviewer-3: `sail-except/Cargo.toml` `[lints] workspace=true` nachtragen.
6. Audit: `license`-Felder (5× BSD-2-Clause, 6× fehlend) nachtragen, kein Build nötig.

## Regeln
E3: KEINE Builds/Tests (kein cargo — Verifikation via Test-Rig). `#![forbid(unsafe_code)]`.
Fremde Logik nicht umbauen, nur die Befunde. Tests als Code (NICHT ausführen).

## Done
- Alle 6 Punkte umgesetzt + committed, Rig-Verifikation offen.
