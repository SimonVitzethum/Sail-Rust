# Lane-24 (O-10, subsidiär): Suite-Flakiness + OOC-Wiederholung — Bericht

Stand: 2026-10-12, Worktree `worktrees/lane-24`, Basis `a972e3a`
(plus nur diese Lane: `scripts/test-memory-limited.sh`, keine Crate-Änderungen).
Alle schweren Läufe über `/tmp/opencode/sail-gate/gated-run.sh 4G`
(ein Envelope, innen `systemd-run` mit 128M/4G-Limits, `MemorySwapMax=0`).
Hüll-Runner: `/tmp/opencode/lane-24-run.sh` (außerhalb des Repos, neu
aufgesetzt nach Host-Reboot — der alte Runner unter `/tmp` war weg).

Nach Host-Reboot (12.10. abends) wurde die komplette Mess-Envelope neu
aufgesetzt und alle Läufe frisch wiederholt (Zahlen s. unten).
Wichtig für Reproduktion: Das neue Gate reicht Env-Variablen NICHT an die
Unit durch (nachgewiesen: `VAR=1 gated-run.sh …` kommt leer an). Daher müssen
Knobs *innerhalb* des Gates gesetzt werden:
`gated-run.sh 4G env SAIL_TEST_THREADS=1 bash scripts/…`
(analog `env SAIL_RS_THREADS=16 …`). Zwei Läufe ohne dieses `env` (Suite mit
Default-t4, OOC mit 8 Workern) wurden erkannt und verworfen/neu gemessen —
die unten stehenden Zahlen sind die verifizierten mit wirksamen Knobs.
Hinweis zum Log-Split bei den Fokusslices: `*.log` enthält die Test-Stdout,
`*.stderr.log` die Gate-Hülle plus cgroup-Peaks (erster `Memory peak` =
Testbinärdatei unter 128M, zweiter = äußere Gate-Hülle).

## 1. Skriptänderung (einzige Datei außerhalb `evidence/`)

`scripts/test-memory-limited.sh` — strikt additiv, abwärtskompatibel:
- Defaults unverändert: `SAIL_TEST_THREADS` default 4, `SAIL_TEST_RETRIES`
  default 0, gleiche Default-Pakete, gleicher Build (`4G`) und gleiche
  Pro-Binärdatei-Limits (`128M`, `RuntimeMaxSec=180`, Binärdateien weiterhin
  nacheinander). Mit Defaults läuft exakt das bisherige Verhalten
  (plus eine `FAIL:`-Zeile im Fehlerfall).
- Neu: `SAIL_TEST_THREADS=1 …` für sequenzielle Diagnose-Re-Runs;
  `SAIL_TEST_RETRIES=1 …` wiederholt fehlgeschlagene Binärdateien einmal mit
  `--test-threads=1` (Parallel-Flake-Diagnose statt Limiterhöhung).
- Doku-Header mit Befundlage (Zahlen s. §2): kein Limit erhöht, nichts
  Bestehendes entfernt.

## 2. Befund: Test-Parallelität, kein Leak — mit Zahlen

Auftrag-Befundlage (39-Test-Binärdatei inkl. uncommittetem mutablem
Tiefentest aus dem Haupt-Worktree, vgl. `evidence/2026-10-12/lane-01-oom-single.log`):
solo ~93,6M ok; sequenziell ~95,5M grün; mit 4 Test-Threads flaky an 128M
(~126,6M ok vs. OOM-Kill).

Eigene Messung auf lane-24-Basis (`a972e3a`, `canonical_program` hier mit
35 Tests — der uncommittete Haupt-Worktree-Tiefentest
`ten_thousand_mutable_and_loop_scopes_…` ist nicht enthalten, daher absolut
kleinere Peaks, aber gleiches Muster). Nach Reboot frisch wiederholt
(Peaks aus `*.stderr.log`, innere Scope = Binärdatei unter 128M):

| Lauf (je 128M, Swap 0) | Ergebnis | cgroup-Peak |
| --- | --- | --- |
| Tiefentest solo (`ten_thousand_nested_vector_literals_…`), `--test-threads=1` | ok (1/1) | 24,1M (vor Reboot 20,4M) |
| `canonical_program`-Binärdatei voll (35 Tests), `--test-threads=1` | ok (35/35) | 53,5M (vor Reboot 53,1M) |
| gleiche Binärdatei, `--test-threads=4`, Versuch 1 | ok (35/35) | 102,7M (vor Reboot 102,4M) |
| gleiche Binärdatei, `--test-threads=4`, Versuch 2 | ok (35/35) | 103,1M (vor Reboot 102,4M) |

Faktor ~1,9× (53,1M → 102,4M) allein durch Test-Parallelität innerhalb einer
Binärdatei. Mit dem schwereren 39-Test-Inhalt (Auftrag-Befundlage) erreicht
die t4-Spitze die 128M-Decke (~126,6M) und kippt je nach Scheduling in den
OOM-Kill — derselbe Mechanismus, kein Leak. Konsequenz im Skript dokumentiert:
erst einzeln/sequenziell unter dem Originallimit messen, nicht erhöhen.

## 3. Vollsuite-Nachweis (stabil konfiguriert)

`SAIL_TEST_THREADS=1 bash scripts/test-memory-limited.sh --workspace`
(via Gate als `gated-run.sh 4G env SAIL_TEST_THREADS=1 …`, s. Hinweis oben)
→ Exit 0. Log: `vollsuite-sequenziell.log` (nach Reboot neu gemessen,
Stdout+Stderr kombiniert, daher Peaks inline).
- 104 Test-Suiten `ok`, insgesamt 1179 Tests bestanden, 0 failed.
- Build: nach Reboot bereits kompiliert (inkrementell, `Finished in 0,04 s`,
  Peak 27M); Vor-Reboot-Vollbuild unter 4G: Peak 771,2M.
- Schärfste Test-Binärdatei: 107,1M (`sail-ast`-Unittests, u.a.
  `storage::…deep_trees…`, 24 Tests) — unter 128M, Swap 0
  (vor Reboot 106,9M, gleiche Suite).
- `canonical_program` darin: 35/35 bei 51,9M in 0,59 s — threads=1 anhand
  Peak+Laufzeit verifiziert (konsistent mit §2; ein versehentlicher
  Default-t4-Lauf ohne `env`-Weitergabe lag bei 102,6M/0,20 s und wurde
  verworfen).

## 4. OOC-Wiederholung mit 16 Workern

`SAIL_RS_THREADS=16 bash scripts/typecheck-ooc-gate.sh`
(via Gate als `gated-run.sh 4G env SAIL_RS_THREADS=16 …`, Workerzahl im Log
verifiziert) → Gate-Pass (Exit 0 des Gate-Skripts).
Log: `ooc-16workers.log` (nach Reboot neu gemessen, Stdout+Stderr kombiniert).
- 4096 Definitionen, 1052672 Ausdrücke, 567.919.927 AST-Bytes auf Platte
  (> 128M-Limit), Typecheck 0,561 s, total 4,38 s, Swap 0
  (vor Reboot: 0,571 s / 6,07 s — gleiche Arbeitslast, reine Laufzeitstreuung).
- Erwarteter Compiler-Exit 1 (fehlende Backend-Integration), kein OOM-Kill.
  Lauf-Peak als 128M gemeldet (Decke berührt, Prozess beendete sich selbst).
- Build-Peak 23,8M (inkrementell, bereits kompiliert; Vor-Reboot-Vollbuild
  257,9M unter 4G-Limit).
- Hinweis zu „nach allen Merges“: Re-Run auf lane-24-Basis; OOC-relevanter
  Code ist dort identisch zu `main`/`a972e3a` (diese Lane ändert keine Crates,
  nur das Test-Skript), daher repräsentativ.

## 5. Abnahme-Einschätzung O-10

- `test-memory-limited.sh`: Vollsuite sequenziell grün (Exit 0, 1179 Tests),
  Default-Verhalten erhalten, Diagnose-Knobs additiv. Erfüllt.
- `typecheck-ooc-gate.sh`: mit 16 Workern neu geloggt und bestanden. Erfüllt.
- Kein Crate-Eingriff, kein Limit erhöht. Die Rest-Flakiness (t4 an 128M bei
  schwerem Inhalt) ist dokumentiert, nicht wegkonfiguriert: Default bleibt 4
  (Reproduzierbarkeit des Befunds), stabil ist `SAIL_TEST_THREADS=1`.

Logs in diesem Ordner: `focused-build.log` (+ `.stderr.log` mit Gate-Hülle),
`focused-solo-threads1.log`, `focused-binary-threads1.log`,
`focused-binary-threads4-try{1,2}.log` (je + `.stderr.log` mit Peaks),
`vollsuite-sequenziell.log`, `ooc-16workers.log` (beide kombiniert, Peaks inline).
