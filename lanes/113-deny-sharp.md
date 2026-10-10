# Lane 113 — deny-sharp-2 (A1–A5 schließen)
Branch: `lane/113-deny-sharp` | Status: AKTIV

## Ziel
AUDIT-Aktionen A1–A5 schließen soweit ohne Build möglich: Lockfile-Policy,
z3-API-Verifikation per Review (Lane-80-Code lesen), Lizenzfelder-Rest,
geiger-Doku. Warnend→scharf wo belegbar.

## Aufgaben
- Klein, präzise, KEINE Builds (E3). Doku + Config.

## Done
- A1: offen, Blocker E3 (kein Lockfile-Lauf) — CI bleibt warnend.
- A2/A3: geschlossen (alle Crates `MIT OR Apache-2.0`, per Review verifiziert).
- A4: geschlossen (`sail-par-pipe` → `0.5.15`, Rest aus Lane 70).
- A5: Code geschlossen (`z3 0.21`, API per Review verifiziert, kein `ApplyResult`); Lauf-Bestaetigung nach A1.
- geiger-Doku: geschlossen (AUDIT.md § 5).
- Schaerfung: alles Belegbare bereits `deny`; Rest bleibt warnend mit Begruendung.
