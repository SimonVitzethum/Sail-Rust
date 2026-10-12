# AGENTS.md — Dauerauftrag Sail-Rust (gilt ab 2026-10-12)

## Auftrag

Vollständiges ARMv9.4-A-Sail-Modell (plus gesamte Sail-Eingabe) mit dem
Rust-Rewrite kompilieren: Sail → vollständige Typprüfung → typisierte
Rewrites → ausführbares Lean → erfolgreicher `lake build` → Prüfung ohne
`sorry` oder andere Sorry-Proof-Holes. Speicher: alles unter
`MemoryMax=16G`/`MemorySwapMax=0`; `28G` ist die ausdrücklich erlaubte
Ausweichgrenze. Ziel ist erst erreicht, wenn `scripts/arm-lean-gate.sh`
grün ist und `scripts/CheckLeanNoSorry.lean` keine Holes findet.

## Arbeitsweise (verbindlich)

- **Weiterarbeiten, basteln, verbessern — über Stunden, selbstständig.**
  Gate laufen lassen → Blocker isolieren (minimale Fixture + OCaml-Vergleich
  mit `/home/simon/.opam/sail/bin/sail -just_check`) → fixen → verifizieren
  → committen → pushen → dokumentieren. Kleine mächtige APIs, zentrale
  Mechanismen, wenig Kopplung, Korrektheit vor Mikrooptimierungen.
- **KEINE Subagents/Agents starten — außer auf explizite Anweisung des
  Nutzers.** Alle Analyse-, Code- und Verifikationsarbeit läuft in dieser
  Session. Hintergrund-Shells für lange Gate-Läufe sind erlaubt (kein Pollen,
  auf Fertigmeldung warten).
- **Alle schweren Läufe über das Gate:** `/tmp/opencode/sail-gate/gated-run.sh
  <MEM> <cmd>` (flock-serialisiert, Start nur bei avail ≥ MEM+2048MiB, sonst
  Exit 99). Immer 2 GiB frei halten (`free -h`/`htop` gegenprüfen — das Gate
  meldet `avail`, nicht `free`). cargo max `-j2`; Tests `--test-threads`
  nach Skript-Default; kein `arm-lean-gate`-Vollauf ohne Freigabe des
  Nutzers — EXPLIZIT: volle ARM-Läufe (16G/28G) sind Teil DIESES Auftrags und
  damit freigegeben.
- **Oft committen und pushen** (`https://github.com/SimonVitzethum/Sail-Rust`,
  Push autorisiert, keine erneute Freigabe nötig). Thematische Commits mit
  Präfix (`grammar:`/`ty:`/`rewrite:`/`solver:`/`lean:`/`docs:` …). Vor jedem
  Commit die betroffene Suite grün via Gate. Kein Reset/Stash/Verwerfen
  fremder Arbeit.
- **Ehrlichkeit vor Grün:** keine Stubs als Erfolg, keine `sorry`, keine
  fabrizierten Beweise (nur UNSAT-implication als Beweis), fehlende Stufen
  mit explizitem Exit-1-Fehler. Jede Erfolgsaussage nennt Prozessausgang +
  Peak. Vermutungen als solche markieren, mit Zahlen belegen oder lassen.
- **Referenz:** OCaml-Semantik in `../sail/src/lib/` (`parser.mly`,
  `initial_check.ml`, `type_check.ml`, `type_env.ml`, `rewrites.ml`);
  Zweifelsfälle per minimaler Fixture gegen OCaml entscheiden.
- **Doku:** `README.md`, `WORK_STATE.md` (Stand + offene Punkte, aktuell
  halten), `ARCHITECTURE.md` (nur Code-wahres mit Pfad:Zeile),
  `TESTING.md` (Befehle), `THIRD_PARTY.md` (Herkunft), plus dieses File.
  Fünf-plus-eins Dateien im Root, keine neuen Dok-Verzeichnisse.
- **Rust-Regeln:** kein `unsafe` (Workspace-`forbid`), `rustfmt` nur auf
  eigene Dateien (Repo-Drift nicht anfassen), Warnungen in eigenen Dateien
  null, keine Duplikat-Module, keine Dead-Code-Ablage ohne Test.

## Nicht tun

- Keine parallelen Worktrees/Lanes ohne Anweisung (Single-Worktree-Arbeit
  im Repo-Root; alte `worktrees/lane-*` nur auf Anweisung anfassen).
- Keine Limiterhöhung als „Fix“ (erst messen: Solo → sequentiell →
  parallel; Ursache benennen).
- Keine Golden-/Fixture-Blind-Updates (nur bei bewusst geprüfter
  Templateänderung).
- `/tmp` ist tmpfs: keine Spool-/Produktionsdaten dorthin
  (`SAIL_RS_SPOOL_DIR` auf echte Platte).
