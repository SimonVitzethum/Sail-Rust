# Lane 67 — deny-audit (Supply-Chain)
Branch: `lane/67-deny-audit` | Status: AKTIV

## Ziel
Supply-Chain-Hygiene: cargo-deny-Config + Advisory-Audit (lesend, keine Builds).

## Aufgaben
- `deny.toml` (licenses MIT/Apache-2.0-only, ban unsound Strict?) + Doku, Audit-Ergebnis
  als Report (Netz-Abfrage ok, KEINE Builds/Tests, E3).
- `#![forbid(unsafe_code)]` wo Code entsteht (Doku-only bevorzugen).

## Done
- deny.toml + AUDIT.md, CI-Einbau als Vorschlag (nicht selbst einbauen).
