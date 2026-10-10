# GATE_STATUS — Multicore-Gate (Lane 97, read-only Audit)

Stand: 10.10.2026 | Methode: nur gelesen + bewertet (E3 — keine Builds/Tests durch diese Lane).
Gate-Quelle: `WAVE3.md` §2 (drei Bedingungen für Lane 41–44 / 58-Nachfolger).
Keine Gate-Änderung — nur Befund.

## 1. Port-durch? — 🔴 ROT

Gate: „E2E-Pipeline Parse→Lean läuft (Lane-45-Fail-Liste abgearbeitet bzw. Fail-Owner bestätigt)".

- `lanes/45-e2e-tiny-arm-fails.md`: F1–F10 alle offen (includes, parse-full-grammar,
  ast-unify/`body_src`-Rohtext, lean-typedefs, lean-bitvec-exprs, lean-int-arith,
  lean-patterns, lean-effects, lean-concurrency, diagnostics) — Skizze, kein Abbau-Nachweis.
- `crates/sail-cli/src/e2e.rs` (Main-Stand): Header „Code-Skizze, E3-Moratorium",
  `run()`/`static_fails()` nirgends verdrahtet. Beleg: Rig-Logs
  `logs/limited-run-20261010-154553-93417.log` + `...-163017-98940.log` melden je
  8× `dead_code` (`E2eFail`/`E2eArgs`/`Outcome`/`run` „never constructed/used").
  `grep e2e::run|tiny_arm crates/sail-cli/src/main.rs` (Main): kein Treffer —
  `mod e2e` existiert, wird aber nie aufgerufen.
- Kein `sail-rs --e2e`-Lauf und kein `lake build` in irgendeinem Rig-Log
  (`logs/`, 11 Dateien, alle vom 10.10.2026).
- `GAP.md` (Lane 75): 37 `.sail`-Dateien / 466.202 Zeilen ARMv9.4-Korpus mit
  37 Lückenklassen (throw 21.014×, `@`-Concat 9.835×, …) — Port-Lücke dokumentiert, nicht geschlossen.

## 2. läuft? — 🔴 ROT

Gate: „E2E-Durchläufe Tiny-Arm (Lane 45) und arm-v8.5-a (Lane 46) sind beim
Integrator mit Memory-Limits gelaufen und protokolliert".

- E2E-Protokolle fehlen vollständig: kein Tiny-Arm-Durchlauf, kein arm-v8.5-a-Durchlauf,
  kein `lake`-Lauf in `logs/`. Die geforderten Integrator-Protokolle existieren nicht.
- Rig-Historie sonst gemischt (kein durchgehend grüner Full-Build):
  - grün (`exit=0`): `...-154553` (`cargo check` ci-Profil, 10,12 s),
    `...-163017` (`cargo check`, 0,32 s, cached), `...-163130`
    (`sail-sve-sme`: 5 + 7 Tests ok), `...-162320`/`...-162326` (triviale Wrapper-Smokes).
  - rot (ohne `exit=0`): `...-154256` (Workspace-Manifest kaputt:
    `sail-lake`-Pfad `/home/simon/Dokumente/crates/...` fehlt), `...-160522` /
    `...-160658` / `...-172218` (je ~33 MB: LALRPOP-Konflikte `sail.lalrpop` —
    `LoopExp`/`AtomicIndexRange`, build-script-Panic), `...-162835`
    (`sail-grammar` `out/sail.rs` fehlt nach gescheiterter Generierung),
    `...-162323` (`sleep`-Executable fehlt). Jüngstes Log (`...-172218`, 17:22 Uhr)
    zusätzlich `E0608`/`E0506` (`sail-ooc`) + `E0015` (`sail-arm-corpus`).
- Teil-Grüns (Checks, 1 Crate-Test) ersetzen die geforderten E2E-Protokolle nicht.
  E3 setzt „Tests grün" als Done-Kriterium aus, das Gate verlangt Integrator-Läufe —
  diese stehen aus.

## 3. RAM-opt? — 🟡 GELB

Gate: „Lane-58-RAM-Optimierung umgesetzt und Peak-Budgets (Lane 14,
`perf/BUDGETS.md`-Format) eingehalten".

- Umsetzung steht (Code gemergt, je E3 ohne Rig-Verifikation):
  - Lane 68 (`aaddbeb`, ACTIVE.md Welle-3): Interner-Einmalspeicherung
    (`Arc<str>`), `subst_cow`, Clone-Diät (`entry`-API, `Arc`+CoW-`TypeEnv`).
    Spec-Status: DONE, „Verifikation via Test-Rig (später)".
  - Lane 71 (`402f7dc`, Welle-4): `TokenKind`-/`LitKind`-/`Kid`-Payloads
    `String` → `Symbol`, Vertrag `docs/payload-intern-contract.md`. Tests als Code, E3.
  - Lane 90 (`90-ramopt-phase2`, nur Worktree `lane-90`, AKTIV, nicht gemergt):
    `simplify`/`internal_lets`-Owned-Rewrites auf `subst_cow`, „Verifikation via Rig".
    (Hinweis: `lanes/` auf Main enthält kein `90-*` — Stand nur über Worktree lesbar.)
- Messung fehlt (Gate-Kern):
  - `perf/BUDGETS.md`: alle drei Modell-Budgets (4/16/24 GiB) auf
    „⏳ Messung ausstehend". Einzige Peaks: 3-Zeilen-Dummy-Fixture (~12 MB, 0,3 %) —
    Datei selbst: „**kein Modell-Budget-Nachweis**".
  - `perf/timings.jsonl`: nur Fixture-Phasen, keine Modell-Phasen.
- GELB, nicht GRÜN: Code-Hebel umgesetzt, aber weder Rig-verifiziert noch Budget-eingehalten.
  Fürs Gate zählt das als nicht erfüllt.

## Fazit (Ampeln)

| Kriterium   | Ampel   | Kurzbeleg |
|-------------|---------|-----------|
| Port-durch? | 🔴 ROT  | F1–F10 offen, `e2e::run` ungenutzt (dead_code), kein `--e2e`-/`lake`-Lauf in Logs |
| läuft?      | 🔴 ROT  | keine E2E-Protokolle (Tiny-Arm/arm-v8.5-a); Rig: Teil-Checks grün, Full-Builds rot (LALRPOP, ooc/corpus-Errors) |
| RAM-opt?    | 🟡 GELB | 68+71 gemergt, 90 aktiv — aber keine Rig-Verifikation, alle Modell-Budgets „Messung ausstehend" |

Gate 41–44 (§2 WAVE3.md): **nicht freigegeben** — keine Bedingung vollständig erfüllt.
Nächste Schritte (Integrator, nicht diese Lane): E2E-Läufe mit Memory-Limits
protokollieren, `lake build`, Modell-Peaks in `perf/BUDGETS.md`-Format messen,
Lane-90-Rig-Verifikation.
