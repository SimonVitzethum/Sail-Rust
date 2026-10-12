# Lane-38 (Suite-Stabilitaet): 128M/4-Thread-Flakiness entscheiden — Bericht

Stand: 2026-10-13, Worktree `worktrees/lane-38`, Basis `a972e3a`.
Nur diese Lane: `scripts/test-memory-limited.sh` (additiv, ein Default-Wechsel
mit Messbeleg) + dieser Ordner. Keine crates/-Aenderungen, kein Limit erhoeht.
Alle schweren Laeufe ueber `/tmp/opencode/sail-gate/gated-run.sh 4G`
(ein Envelope, innen `systemd-run` mit 128M/4G-Limits, `MemorySwapMax=0`).
Haupt-Worktree nur gelesen (Referenz: Lane-24-Skript mit
SAIL_TEST_THREADS/RETRIES, REPORT mit solo 93,6M / sequenziell 95,5M /
4-Threads flaky ~126M/OOM).

## 1. Messreihe: canonical_program-Binaerdatei t1/t2/t4 je 3x unter 128M

Binaerdatei: `target/ci/deps/canonical_program-ddfe03c20e391be8` (35 Tests,
Basis `a972e3a`; der uncommittete Haupt-Worktree-Tiefentest ist nicht enthalten).
Je Lauf: Gate-Envelope 4G + innere Scope 128M/Swap 0 (`systemd-run --wait`,
Peaks aus `*.stderr.log`, erster `Memory peak` = Testbinaerdatei).
Build einmalig unter 4G (`build.log`, Peak 994,2M).

| Lauf (je 128M, Swap 0) | Ergebnis | cgroup-Peak (Binaerdatei) | Zeit |
| --- | --- | --- | --- |
| t1 try1 `--test-threads=1` | ok (35/35) | 53,0M | 0,64 s |
| t1 try2 | ok (35/35) | 53,0M | 0,60 s |
| t1 try3 | ok (35/35) | 52,4M | 0,61 s |
| t2 try1 `--test-threads=2` | ok (35/35) | 75,1M | 0,34 s |
| t2 try2 | ok (35/35) | 65,6M | 0,35 s |
| t2 try3 | ok (35/35) | 74,5M | 0,35 s |
| t4 try1 `--test-threads=4` | ok (35/35) | 103,0M | 0,20 s |
| t4 try2 | ok (35/35) | 102,3M | 0,20 s |
| t4 try3 | ok (35/35) | 102,8M | 0,20 s |

Befund:
- t1 stabil bei ~53M (75M Abstand zur Decke), aber 3x langsamer als t4.
- t2 stabil gruen, max 75,1M (~53M Abstand), 1,7x schneller als t1.
  Spannweite 65,6–75,1M (~9,5M Scheduling-Varianz) bleibt weit unter der Decke.
- t4 stabil gruen auf dieser 35-Test-Basis, aber max 103M (nur ~25M Abstand);
  Faktor t4/t1 ~1,9x. Mit schwererem 39-Test-Inhalt erreicht die t4-Spitze die
  Decke (~126,6M ok vs. OOM-Kill, Auftrag-Befundlage) — derselbe Mechanismus,
  kein Leak. Varianz belegt: Threadzahl allein bewegt den Peak um ~50M.

## 2. Entscheidung: genau eine Option

Gewaehlt: **(a) SAIL_TEST_THREADS-Default auf 2 senken** (`scripts/test-memory-limited.sh`:
`TEST_THREADS="${SAIL_TEST_THREADS:-2}"`, sonst unveraendert — 128M/4G/Jobs 2/
Retries 0/Default-Pakete bleiben, Override via Env bleibt).
Beleg: t2-Max 75,1M = -27% gegenueber t4-Max 103M bei 1,7x Tempo gegenueber t1;
3x gruen unter 128M. Limit unangetastet.

Verworfen:
- **(b) Binaerdatei-Split (canonical_program in 2 Targets)**: braucht
  `crates/sail-ty`-Aenderungen (Cargo-Targets + Testdatei) — der Auftrag
  verbietet crates/-Aenderungen. Zudem adressiert der Split die Ursache
  (parallele Peak-Addition) nur indirekt und verdoppelt Build-/Pflegeaufwand.
- **(c) 128M -> 160M**: keine Budget-Doku-Rechtfertigung — das OOC-Byte-Budget
  (Default 32M vs. 128M-Gate-Workload, `crates/sail-ooc/src/budget.rs`)
  betrifft das Streaming-Fenster gleichzeitiger Definitionen, nicht die
  Testthread-Parallelitaet; es begruendet keinen 160M-Testwert. Verstoesst zudem
  gegen TESTING.md („erst betroffene Binaerdatei einzeln unter dem
  Originallimit messen; ein hoeheres Limit beseitigt den Befund nicht“) und
  gegen „keine Limiterhoehung ohne Messung“ (die Messreihe zeigt gerade, dass
  128M mit t2 bequem reicht).

## 3. Skriptaenderung (einzige Datei ausserhalb `evidence/`)

`scripts/test-memory-limited.sh` — Lane-24-Knoepfe (THREADS/RETRIES) uebernommen,
einziger Default-Wechsel 4 -> 2 mit Messbeleg (§1). Diff gegen `a972e3a`:
Header-Doku (Lane-24 + Lane-38), `TEST_THREADS`/`TEST_RETRIES`-Variablen,
Retry-Schleife (einmal `--test-threads=1`), `FAIL:`-Zeile. Kein Limit erhoeht,
nichts Bestehendes entfernt.

## 4. Abnahme: 3x aufeinanderfolgende gruene Wrapper-Laeufe via Gate

Scope: `-p sail-ty --test canonical_program` (die vermessene Binaerdatei),
mit neuem Default (kein `SAIL_TEST_THREADS` gesetzt, Gate reicht Env ohnehin
nicht durch — Default 2 wirkt). Je Lauf Gate-Envelope 4G:

- `wrapper-default-try1.log`: Exit 0, 35/35, Peak 64,1M (s. `.stderr.log`)
- `wrapper-default-try2.log`: Exit 0, 35/35, Peak 69,2M
- `wrapper-default-try3.log`: Exit 0, 35/35, Peak 69,3M

Logs in diesem Ordner: `build.log` (+ `.stderr.log` mit Gate-Huelle),
`canonical-threads{1,2,4}-try{1,2,3}.log` (je + `.stderr.log` mit Peaks),
`series-progress.log` (kompakte Peak-Tabelle), `wrapper-default-try{1,2,3}.log`
(je + `.stderr.log`).
