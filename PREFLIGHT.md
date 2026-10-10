# PREFLIGHT — Startbereitschaft ARM-Lauf `arm-v9.4-a` (Lane 93)

Stand: 10.10.2026, ca. 17:35 UTC. E3: kein `make lean`, keine Last.
Nur gelesen + zwei lastfreie Smokes (`limited-run -- true`,
Wrapper-`--dry-run`). Außerhalb des Worktrees nichts verändert.

## 1. trace.sail-Stub-Stand

- `src/prelude.sail:46` verlangt `$include <trace.sail>`.
  Weder Sail 0.20.3 noch sail2-HEAD liefern die Datei (Upstream-Drift).
- Stub **vorhanden** an beiden relevanten Stellen (jeweils nur lesend geprüft):
  - `~/.opam/sail/share/sail/lib/trace.sail` (340 B, kommentar-only, leer,
    angelegt heute 15:53 UTC) — das ist der Pfad, den `sail-voll-lean.sh`
    über `sail` aus PATH + Makefile-`SAIL_DIR:=opam-var` trifft.
  - `/home/simon/Dokumente/gabbro-arm/sail-share/lib/trace.sail` (89 B,
    `// intentionally empty …`, seit 07.10.) — trifft nur `bin/sail-lean.sh`
    (`SAIL_DIR=sail-share`, Legacy-Pfad, nicht der geplante Lauf).
- Stub-Genügsamkeit per `grep` begründet: `TraceSynchronizationBarrier`,
  `HaveSelfHostedTrace` u. a. sind **im Modell definiert**
  (`src/v8_base.sail:18571ff`, `:39803ff`); übrige `Trace…`-Treffer sind
  String-Literale (`impdefs.sail:82,118`). Kein `$include`-Konsument außer
  `prelude.sail` referenziert Symbole aus `trace.sail`.
- **Restrisiko (offen):** heutiger 15:49-Lauf
  (`gabbro-arm/tmp/sail-voll/lauf-20261010-154910.log`) scheiterte exakt daran:
  `Fatal error: … realpath …/lib/trace.sail (ENOENT)`, Exit 2, kein
  `lean.stamp`. Der Stub kam erst 15:53 — **seitdem kein Lauf**,
  d. h. „leerer Stub wird akzeptiert“ ist noch nicht belegt.
  Die ersten Minuten des Rig-Laufs validieren genau das (bei Ablehnung:
  Stub mit echten `val`-Deklarationen füllen, Owner Haupt-Session).
- Vendored-Rust-Prelude (`crates/sail-prelude`, 26 Dateien, Sail 0.20.3,
  Lane 18/81) enthält kein `trace.sail` — betrifft nur den Rust-Pfad,
  nicht diesen OCaml-Lauf.

## 2. Skript-Review (nur gelesen)

- `gabbro-arm/sail-voll-lean.sh` (der Lauf): `sail` aus
  `$HOME/.opam/sail/bin` (PATH-Präfix), `OCAMLRUNPARAM=o=60`,
  `TMPDIR` auf SSD-Scratch (`tmp/sail-voll/tmp-<ts>`, weg von `/tmp`-tmpfs),
  `/usr/bin/time -v make lean -j1`, Log nach `tmp/sail-voll/lauf-<ts>.log`,
  `lean.stamp`-Meldung am Ende. Kein eigenes Memory-Limit (kommt vom Wrapper).
  Kopfkommentar verlangt „Swap ≥ 16G“ — **stale**, Maschine hat 0 B Swap
  (s. §5); mit Wrapper (`MemorySwapMax=0`) irrelevant, ohne Wrapper riskant.
- `gabbro-arm/sail-voll-lean-limited.sh` (20G-Wrapper): Default `-m 20G`,
  `-s arm-run`, Format-Check `[0-9]+[MG]`, `--dry-run` funktioniert
  (Smoke heute ok), Scope `MemoryMax=20G + MemorySwapMax=0`, tmux-Fenster
  `lean-<ts>`. Sauber; einzige Anmerkung: kein `remain-after-exit` wie
  `tmux-test.sh` (Scope stirbt mit Caller — für tmux-Fenster ok).
- `gabbro-arm/bin/sail-lean.sh` (Legacy): `SAIL_DIR=sail-share`,
  `OCAMLRUNPARAM=o=20`, `make gen_lean` — **nicht** der geplante Lauf
  (anderes Target, keine Vermessung, kein Limit). Nicht verwenden.
- `gabbro-arm/bin/oom-waechter.sh`: killt bei < 2,5 GB verfügbarem RAM erst
  `lean/lake/cargo/rustc`, dann opencode-Agenten; killt **nie** `sail`.
  Sinnvoller Backstop, ersetzt kein MemoryMax.
- Rig im Worktree (`scripts/limited-run.sh`, `scripts/tmux-test.sh`,
  `docs/TEST-RIG.md`, Lane 51): Smokes heute ok
  (`limited-run -- true` → exit=0; `tmux-test --dry-run` → saubere Zeile).
  Test-Hinterlassenschaft (`logs/…`) wieder entfernt. Für den OCaml-Lauf
  gilt der Gabbro-Wrapper, nicht diese Skripte (nur Doku-Referenz).

## 3. Checkliste Switch / Pfade / lean-Ziel / Platte

- [x] **Switch:** `opam switch list` → `sail (ocaml-system.5.5.0)` aktiv (→).
  Aber: `[WARNING] environment is not in sync` — Starter muss
  `eval $(opam env --switch=sail)` ausführen (Skripte setzen nur PATH;
  das Makefile braucht `opam config var`, deprecated-Warnung harmlos).
  `sail --version` → `Sail 0.20.3`. `lean`/`lake` via elan vorhanden
  (nur für Später, dieser Lauf braucht sie nicht).
- [x] **Pfade — Laufquelle:** `sail-voll-lean.sh` nutzt
  `gabbro-arm/src/sail-arm/arm-v9.4-a` (eigener Checkout, 165 MB).
  `diff -rq src/` gegen `Dokumente/Sail/sail-arm/arm-v9.4-a/src/` → **identisch**;
  beide `prelude.sail` byte-identisch. `Sail/sail-arm` ist Git-Repo
  (HEAD `aacf240`), gabbro-Kopie ohne `.git`-Kennung im Modelldir
  (kein `.git` unter `arm-v9.4-a/`) — Drift nach Laufstart vermeiden,
  nur in **einer** Kopie laufen (Empfehlung: gabbro-Kopie, da Skript-Pfad).
- [x] **Pfade — Logs/Temp:** `LOGDIR=gabbro-arm/tmp/sail-voll`
  (aktuell 316 MB, ein Alt-Log + ein `tmp-*`), `TMPDIR` pro Lauf neu auf
  SSD-Scratch. `/tmp` ist 16-GB-tmpfs → Redirect im Skript ist Pflicht
  und vorhanden.
- [x] **lean-Ziel:** `lean/` enthält in beiden Checkouts **nur**
  `ArmExtras.lean` (8 KB, Platzhalter-Imports `THE_MODULE_NAME…`, wird vom
  Emit ersetzt), **kein** `lean.stamp` — sauberes Vorfeld.
  Makefile-`gen_lean` (Z.85–97) passt zu RUNBOOK §3.3
  (`-dprofile -memo_z3 -lean --lean-force-output -lean_output_dir lean
  -o armv9 -undefined_gen -lean-noncomputable -lean-line-width 100000
  -lean-import-file lean/ArmExtras.lean` über `$(SAIL_SRCS) +
  src/termination_measures.sail`, danach `touch lean.stamp`).
  Achtung RUNBOOK §4: `--lean-force-output` verwirft `lean/` — ok, da nur
  `ArmExtras.lean` drin (Versioniertes, regenerierbar aus Git).
- [x] **Platte:** `/` 274 GB frei (69 % belegt) — mehr als genug.
  `logs/`/`target/`-Disziplin betrifft nur sail-rs, nicht diesen Lauf.
- [Δ] **RAM/Swap:** 31 GiB gesamt, ~24 GiB verfügbar, **0 B Swap**.
  20G-Deckel + `MemorySwapMax=0` schützt die Maschine mit Reserve;
  Budget (24 GiB, `BUDGET_ARM_V94A_GB`) einhaltbar, wenn der Lauf ≤ 20G bleibt.
  15:49-Log zeigt Pre-Stub-Phase mit 25 GiB verfügbar — unauffällig.
  Swap-Warnung im Skriptkopf ignorieren (stale, s. §2).

## 4. Startbefehl für die Haupt-Session (Rig, E3-Ende)

```sh
eval $(opam env --switch=sail)
MEM=20G SESSION=arm-run /home/simon/Dokumente/gabbro-arm/sail-voll-lean-limited.sh
# Beobachten: tmux attach -t arm-run
# (Abhängen: Ctrl-b d; Log: gabbro-arm/tmp/sail-voll/lauf-<ts>.log)
```

Erfolg = `lean.stamp` in `gabbro-arm/src/sail-arm/arm-v9.4-a/` + Exit 0.
Bei `exit=137` (OOM): wie Budgetriss behandeln, nicht Deckel erhöhen
(RUNBOOK §2). `oom-waechter.sh` parallel starten optional.

## 5. GO / NO-GO

**GO (mit einem dokumentierten Restrisiko):** Switch aktiv, Sail 0.20.3,
Stub an beiden Lib-Orten, Skripte reviewed (Dry-Run + Rig-Smokes ok),
`lean/` sauber, `ArmExtras.lean` vorhanden, SAIL-Quellen beider Checkouts
identisch, 274 GB Platte frei, 20G-Deckel bei 24 GB verfügbarem RAM solide.

**Einzige offene Wette (§1):** leerer `trace.sail`-Stub post-Fix noch nie
durch einen Lauf belegt. Fällt `sail` in den ersten Minuten darüber
(statt über Modell-Fehler), Lauf abbrechen und Stubinhalt eskalieren —
kein Maschinenrisiko, nur Agenda-Risiko.
