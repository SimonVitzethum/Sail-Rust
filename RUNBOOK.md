# RUNBOOK — 20GiB-ARM-Lauf (`arm-v9.4-a`, Lane 85)

Ziel: `sail-arm/arm-v9.4-a` (464 212 Zeilen, 19,3 MB, Stand 10.10.2026)
mit dem Rust-Binary (`sail-rs`, Lane-38-Drop-in) bis `lean.stamp` treiben —
multithreaded-fähig, speicherbegrenzt, jederzeit abbrech-/fortsetzbar.
**Dieser Lauf gehört der Haupt-Session im Rig; diese Lane führt nichts aus (E3).**

Quellen: `lanes/31/32/33-*` (Shard-Reports), `perf/BUDGETS.md` + `docs/BUDGETS_ALL.md`
(Budgets), `docs/TEST-RIG.md` + `scripts/limited-run.sh` + `scripts/tmux-test.sh`
(Rig), `lanes/51-test-rig.md`, `perf/arm-v8.5-a-plan.md` (Messmuster),
OCaml-Referenz `sail-arm/arm-v9.4-a/Makefile` (`gen_lean`-Target).

## 0. Voraussetzungen (keine Last, darf jede Session prüfen)

```sh
systemd-run --user --version   # user scope verfügbar?
tmux -V                        # tmux installiert? (Ref: 3.8)
./scripts/limited-run.sh -- true          # Smoke, muss exit=0 melden
./scripts/tmux-test.sh --dry-run -- true  # Smoke, zeigt nur tmux-Zeile
free -g; swapon --show            # Zielmaschine: ~32 GiB RAM, kein Swap
cargo --version; rustc --version
ls sail-arm/arm-v9.4-a/src/*.sail | wc -l  # Modell vorhanden?
```

Referenz-Umgebung: systemd 262, tmux 3.8, `rustc 1.97.1` (Lane-31-Messung).

**Eiserne Regeln (Rig + E3):**

1. Kein `cargo test`/`cargo build`/`sail-rs`/`lake` im Vordergrund der
   Chat-Session — **immer** `tmux-test.sh` + `limited-run.sh`.
2. Ein Fenster je Run, Session = Kampagne (`arm20`). Keine parallelen
   Schwerläufe aus mehreren Sessions; vor neuem Builder-Lauf alte beenden.
3. `logs/` ist Wegwerfware (nicht committen, wie `target/`).

## 1. Shard-Plan (`arm-v9.4-a`, `src/`)

Reihenfolge = `SAIL_SRCS`-Ordnung aus dem OCaml-`Makefile` (zuzüglich
`src/termination_measures.sail` am Ende). Kein Shard überlappt; Shard-Grenzen
aus `file_start`/`file_end` (Lane 11), Fingerprints pro Shard, max. 8 Shards
inflight, Parallel-Scheduler über Shards (Owner Lane 11).

Quelle der Zeilenzahlen: `wc -l` auf `sail-arm/arm-v9.4-a/src/*.sail`.

| Shard | Dateien | Zeilen ca. | Herkunft / Bemerkung |
|-------|---------|------------|----------------------|
| S0-base | `prelude` (375) + `decode_start` (89) + `builtins` (208) + `stubs` (463) + `interrupts` (368) + `interface` (199) + `devices` (286) + `impdefs` (900) + `mem` (232) | ~3 100 | winzig, ein Shard |
| S1-sysregs | `sysregs_autogen` (160 302) | ~160 300 | Lane 31: 5 630 Top-Level-Defs = 2 815 `val`/`function`-Paare, strikte Alternation; Streaming-Split, Standard `defs_per_block=64` → 88 Blöcke; Peak nur ~3 MiB (reiner Scan, §4) |
| S2-sysrest | `sysregs` (1 481) + `reset` (1 466) | ~2 900 | klein, zusammen |
| S3-v8base | `v8_base` (57 139) | ~57 100 | Lane 33: 7 581 Defs (2 663 `val`, 2 427 `function`, 2 491 Helfer-Köpfe); stellt alle größten Einzel-Defs (`FPExpCoefficient_read` 500 Zl./163 Arme, `InitFeatureImpl`, `AArch32_S1Walk*`, …) |
| S4-i32 | `instrs32` (48 015) | ~48 000 | Lane 33: 4 347 Defs; 1 127 `decode_*`→`execute_*`-Kanten 1:1:1; größter Beitrag `__DecodeT32` (513 Clauses) |
| S5-i64 | `instrs64` (53 106) | ~53 100 | Lane 33: 3 766 Defs; 1 036 Kanten; größter Beitrag `__DecodeA64_DataProcFPSIMD` (575); tiefste `if`-Schachtelung in MCPY/MSET-`execute_*` (Terminierungsrisiko Lane 26) |
| S6-sve | `instrs64_sve` (95 548) | ~95 500 | Lane 32: 2 442 `val` + 3 663 `function`; 1 411 `foreach`-Sites, `__DecodeA64_SVE` 1 161 Arme (größte Lean-Funktion gesamt) |
| S7-sme | `instrs64_sme` (42 546) | ~42 500 | Lane 32: 1 110 `val` + 1 665 `function`; geschachtelte Tile-`foreach` + `ZAtile_*` (77 Sites); `__DecodeA64_SME` 617 Arme |
| S8-tail | `decode_end` (111) + `fetch` (389) + `map_clauses` (465) + `event_clauses` (138) + `termination_measures` (116) | ~1 200 | enthält Default-Arme + Top-Dispatch `__DecodeA64`/`__DecodeA32` (`decode_end`), gestreute `__Decode…`-Deklarationen kommen aus `decode_start` (S0) |

Summe S0–S8 ≈ SAIL_SRCS + `termination_measures.sail`. Nicht Teil des
Lean-Emit-Laufs (nur Executable-/Alternativ-Varianten via `Makefile`):
`elfmain.sail`, `no_devices.sail` (Alternativ-`DEVICES`), `isla_main`/`isla_splice`.

**Extra-Split-Kandidaten** (erst bei Budgetriss, Owner Lane 11):
S1 (`defs_per_block` 64→16: 352 Blöcke, Peak −0,2 MiB gemessen),
S6/S7 (Hälften nach Top-Level-Defs), S4/S5 (instrs-Hälften),
S3 (`FPExpCoefficient_read` isolieren). Exakte Schnitte per
`file_start`/`file_end`-Scan beim Lauf.

**Bekannte Fail-Last** (kein Abbruchgrund, sondern Katalog für Owner-Lanes):
S6+S7: `loop_lowering` 2 751 (Lane 27), `constraint_solving` ~2 161
(Lane 04/05), `exception_semantics` ~2 604 (Lane 28), `extern_mapping` ~14 870
(Lane 25), `vl_dispatch` 1 784 (Lane 20/21), `undefined_init` 2 595
(Lane 09/22) — Spans in `crates/sail-sve-sme/FAILURES_SVE_SME.md` §4.
S4+S5: `decode_max_sveveclen`/`decode_max_sv eveclen` fallen per Namensregel in
`decode`-Shard, bei Emission als Helfer behandeln (`CORE_REPORT.md` §2).

## 2. Phasenfolge mit Budgets

Pro Shard × Phase je ein JSONL-Record (`perf/timings.jsonl`-Format):

```
parse → resolve → typecheck → lean-emit
```

Messkette wie Lane 46: `perf/measure.sh` für Einzelprozesse (GNU-`time -v`
bevorzugt, Fallbacks eingebaut), `perf/measure-armv85.sh`-Muster (`/proc`-
Polling von `VmHWM` über Kind + Nachkommen) für mehrstufige Treiber.
Namensschema `<shard>-<phase>`, z. B. `s1-sysregs-parse`;
`note`: `model=arm-v9.4-a shard=S1-sysregs host=<arch>`.

Verbindliches Modell-Limit (Lane 14, `crates/sail-perf/src/lib.rs`:
`BUDGET_ARM_V94A_GB = 24`, `BUDGETS`-Array, `check_budget()`):

| Modell | Limit | Limit (KiB) | Rig-Deckel |
|--------|-------|-------------|------------|
| `arm-v9.4-a` | 24 GiB | 25 165 824 | **20G** (`limited-run.sh -m 20G`) — der Rig-Deckel ist schärfer als das Budget; wer ihn hält, hält das Budget mit ~4 GiB Reserve |

Messmatrix (36 Records: 9 Shards × 4 Phasen), jeder Peak gegen
25 165 824 KiB prüfen (`check_budget()`-Semantik: `peak <= limit` ✅,
sonst ❌ + Eskalation):

| Shard | parse | resolve | typecheck | lean-emit |
|-------|-------|---------|-----------|-----------|
| S0-base | ⏳ | ⏳ | ⏳ | ⏳ |
| S1-sysregs | ⏳ (Scan-Referenz: 3 108 KiB ✅) | ⏳ | ⏳ | ⏳ |
| S2-sysrest | ⏳ | ⏳ | ⏳ | ⏳ |
| S3-v8base | ⏳ | ⏳ | ⏳ | ⏳ |
| S4-i32 | ⏳ | ⏳ | ⏳ | ⏳ |
| S5-i64 | ⏳ | ⏳ | ⏳ | ⏳ |
| S6-sve | ⏳ | ⏳ | ⏳ | ⏳ |
| S7-sme | ⏳ | ⏳ | ⏳ | ⏳ |
| S8-tail | ⏳ | ⏳ | ⏳ | ⏳ |

**Bei Budgetriss** (Wortlaut wie `check_budget()`-Fehlertext): Shard
verkleinern (Extra-Split, Owner Lane 11) oder Pass in Lane 07/09 zurückweisen.
Fail-Liste mit je max. 1 Owner an die Owner-Lane zurückmelden. Peaks danach in
`perf/BUDGETS.md` („Gemessene Peaks“) + `perf/timings.jsonl` nachtragen,
Status oben auf ✅/❌ setzen. **OOM-Kill durch den Rig-Deckel (Exit 137)
zählt wie ein Budgetriss** — erst Shard splitten, nicht Deckel erhöhen.

## 3. Exakte Rig-Befehle

Alle schweren Befehle laufen als: `tmux-test.sh -s <session> -w <fenster>
-m <mem> -- <befehl>`. Session für diese Kampagne: **`arm20`**.
Log folgt immer aus der letzten `limited-run:`-Zeile
(`limited-run: exit=N log=…`).

### 3.1 Kampagne anlegen + Binary bauen (Builder-Profil, 20G)

```sh
# Session anlegen + Release-Binary bauen (ein Fenster je Run):
./scripts/tmux-test.sh -s arm20 -w build -m 20G -- cargo build --release -p sail-cli
# Status beobachten (Chat-Session bleibt frei, Ctrl-b d hängt ab):
tmux capture-pane -p -t arm20:build
tail -f logs/limited-run-*.log   # Logpfad aus der tmux-test-Ausgabe nehmen
```

`.cargo/config.toml` deckelt bereits `[build] jobs = 4` (RAM-Schutz).

### 3.2 Shard-Läufe (je Shard × Phase, 20G, mit Peak-Protokoll)

Treiber-Flags folgen aus Lane 38 (`--lean`, `--lean-output-dir`,
`--lean-force-output`, `-o`, `--memo-z3`-Äquivalent bounded, `-dprofile`-
Timings, `-v`); OCaml-Referenz für den Voll-Lauf (§3.3). Shard-Auswahl via
`--shard` (Lane-11-Schnittstelle; falls noch nicht verdrahtet: Shard-Dateien
explizit als Eingaben übergeben, Reihenfolge §1 einhalten).

```sh
# Muster je Shard x Phase (S1 gezeigt, S0-S8 analog):
SAIL_PERF_NOTE="model=arm-v9.4-a shard=S1-sysregs host=$(uname -m)" \
./scripts/tmux-test.sh -s arm20 -w s1-parse -m 20G -- \
  bash perf/measure.sh --phase s1-sysregs-parse --log perf/timings.jsonl -- \
    ./target/release/sail-rs --shard S1-sysregs --phase parse -- <sharddateien>
# Alternativ für mehrstufige Treiber (Prozessbaum-Peak):
SAIL_PERF_NOTE="model=arm-v9.4-a shard=S1-sysregs host=$(uname -m)" \
./scripts/tmux-test.sh -s arm20 -w s1-parse -m 20G -- \
  bash perf/measure-armv85.sh --phase s1-sysregs-parse --log perf/timings.jsonl -- \
    ./target/release/sail-rs --shard S1-sysregs --phase parse -- <sharddateien>
```

Reihenfolge: S0→S8 in `sail.json`-/`SAIL_SRCS`-Ordnung; S1/S6/S7 (die drei
Riesen) einzeln, nie parallel zueinander. Fenstername = `<shard>-<phase>`
(z. B. `s6-sve-typecheck`), damit `tmux ls` den Fortschritt zeigt.

### 3.3 Voll-Lauf bis `lean.stamp` (OCaml-`gen_lean`-Äquivalent, 20G)

Referenz (`sail-arm/arm-v9.4-a/Makefile`, `gen_lean`-Target): SAIL läuft mit
`-dprofile -memo_z3 -lean --lean-force-output -lean_output_dir lean
-o armv9 -undefined_gen -memo-z3 -lean-noncomputable -lean-line-width 100000
-lean-force-output -lean-import-file lean/ArmExtras.lean` über
`$(SAIL_SRCS) src/termination_measures.sail`, danach `touch lean.stamp`.
Rust-Drop-in (Lane 38: `sail`-CLI-kompatibel, `-`/`--`-Präfix äquivalent,
`--flag=wert` ≡ `--flag wert`):

```sh
# Im Modellverzeichnis (sail-arm/arm-v9.4-a), Ausgabe nach lean/:
./scripts/tmux-test.sh -s arm20 -w lean-full -m 20G -- \
  bash perf/measure.sh --phase full-lean-emit --log perf/timings.jsonl -- \
    /pfad/zu/sail-rs -dprofile -memo-z3 \
      --lean --lean-force-output \
      --lean-output-dir lean -o armv9 \
      --undefined-gen --memo-z3 \
      --lean-noncomputable --lean-line-width 100000 \
      --lean-import-file lean/ArmExtras.lean \
      src/prelude.sail src/decode_start.sail src/builtins.sail src/v8_base.sail \
      src/stubs.sail src/interrupts.sail src/interface.sail src/devices.sail \
      src/impdefs.sail src/mem.sail src/sysregs_autogen.sail src/sysregs.sail \
      src/reset.sail src/instrs32.sail src/instrs64.sail src/instrs64_sve.sail \
      src/instrs64_sme.sail src/decode_end.sail src/fetch.sail \
      src/map_clauses.sail src/event_clauses.sail src/termination_measures.sail
# Erst bei Erfolg (exit=0 + §5-Kriterien erfüllt):
touch lean.stamp
```

`SAIL_PERF_NOTE="model=arm-v9.4-a shard=full host=$(uname -m)"` für den
Record setzen. `lean/ArmExtras.lean` muss existieren (Makefile-Dependency).
`--lean-force-output` überschreibt `lean/` — nie mit ungesichertem
Vorlauf kombinieren (§4).

### 3.4 Leichte Checks (Checks-Profil, 10G, eigenes Fenster)

```sh
./scripts/tmux-test.sh -s arm20 -w checks -m 10G -- ./scripts/check.sh
./scripts/tmux-test.sh -s arm20 -w sorry -m 10G -- \
  grep -rn "sorry" lean/ crates/sail-lake/golden/ || echo "sorry-frei"
```

## 4. Abbruch-/Resume-Punkte

| Punkt | Lage | Fortsetzen mit |
|-------|------|----------------|
| A. Nach `build` | `target/release/sail-rs` vorhanden | weiter bei §3.2 S0 |
| B. Je Shard × Phase | Record in `perf/timings.jsonl` (`<shard>-<phase>`) | nur fehlende Records nachfahren; fertige Shards nicht wiederholen |
| C. Nach allen Shard-Phasen | 36 Records vollständig, alle ✅ | weiter bei §3.3 |
| D. Nach `lean-full` exit=0 | `lean/` befüllt, `lean.stamp` gesetzt | nur noch §5 verifizieren |
| E. Abbruch irgendwo | Fenster lebt weiter / Log zeigt `exit≠0` | s. unten |

**Abbrechen** (OOM-Verdacht, falsche Flags, Kampagne neu starten):

```sh
tmux capture-pane -p -t arm20:<fenster>   # erst Lage sichern
tmux kill-window -t arm20:<fenster>       # einzelnen Run beenden
# oder alles:
tmux kill-session -t arm20
# Rig-Unit (falls Session hängt): systemctl --user stop tmux-arm20
```

**Resume-Regeln:**

- Detroit nie: `limited-run`-Log nennt Exit-Code in der letzten Zeile
  (`limited-run: exit=N`). Exit 137 = OOM-Kill → wie Budgetriss behandeln
  (§2), Shard splitten, Einzel-Fenster wiederholen.
- `--lean-force-output` löscht Zielinhalt: Resume von D **nur** wenn `lean/`
  verworfen werden darf; sonst ohne das Flag in ein frisches Verzeichnis
  emittieren und diffen (Lane 13).
- Shard-Fingerprints (`combined_fingerprint`, Lane 31) vor/nach Resume
  vergleichen — ungleicher Fingerprint bei gleicher Konfig = Eingabe hat sich
  geändert, Shard von vorn.
- `logs/` vor Wiederholung notieren (Zeitstempel + Exit-Code ins
  Peak-Protokoll), alte Logs nicht löschen.

## 5. Erfolgskriterien (alle drei kumulativ)

1. **`lean.stamp`**: `lean.stamp` im Modellverzeichnis existiert, `lean/`
   enthält das Emit-Ergebnis (`<Camel>/Defs.lean`, `<Camel>.lean`,
   `lakefile.toml`, `lake-manifest.json`, `lean-toolchain` — Layout siehe
   `crates/sail-lake/CONTRACT.md`).
2. **sorry-frei (E2)**: `grep -rn "sorry" lean/` → null Treffer; ebenso
   `crates/sail-lake/golden/`. Harte Fehler mit Span statt stillem `sorry`
   sind das erwartete Verhalten (`sail-lean`, `sail-lake`, Lane 52: E3512) —
   offene Bodies gehören in die Fail-Liste (§1), nicht in den Output.
3. **Peak-Protokoll**: alle 36 Records in `perf/timings.jsonl`
   (`{"phase","wall_ms","peak_rss_kb","note"}`-Format, lesbar via
   `sail_perf::read_records`), jeder Peak via `check_budget()` gegen
   25 165 824 KiB geprüft; Ergebnis in `perf/BUDGETS.md`-Tabelle
   („Gemessene Peaks“) + `docs/BUDGETS_ALL.md` §2 nachgetragen
   (Status ✅/❌ statt ⏳). Rig-Deckel-Einhaltung (20G, kein Exit 137)
   im Protokoll vermerkt.

Abnahme: Haupt-Session kann diesem Runbook blind folgen; Lane-85-Done =
dieses Dokument ist abnahmefähig (keine Läufe in dieser Lane).
