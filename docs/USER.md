# USER.md — Benutzer-Doku (Lane 119, Ist-Stand, nur abgeschrieben)

Stand: Worktree `lane-119` (Branch `lane/119-user-docs`), 10.10.2026.
Methode: rein lesend aus Code/Skripten/Doku zusammengeschrieben (E3:
keine Builds/Tests ausgeführt). Jede Aussage nennt ihre Quelle in
Klammern; wo eine Quelle nur ein Vorschlag ist, steht das dabei.

## 1. Bauen mit Profilen

Voraussetzung: Rust-Toolchain **>= 1.75.0** (MSRV, `Cargo.toml`
`[workspace.package] rust-version`; CI prüft mit genau dieser Toolchain
im `msrv`-Job) (`Cargo.toml:49-55`, `INSTALL-RUST.md`, `ci.yml:138-145`).
Für den Lean-Pfad sind **weder opam noch dune noch OCaml** nötig
(`INSTALL-RUST.md`).

Profile im Workspace-`Cargo.toml` (`Cargo.toml:64-76`):

```toml
[profile.release]   # lto = "thin", strip = true, panic = "unwind"
[profile.ci]        # inherits = "release", lto = false, debug = false
```

- `release`: speicherschonend (thin-LTO statt fat, keine Debuginfos) —
  Kommentar in `Cargo.toml:64-66` nennt den Grund (Swap-lose Maschine, E3-Folge).
- `ci`: erbt von `release` (Opt-Level etc.), aber ohne LTO und ohne
  Debuginfos — für speicherschonende Test-Binaries (`Cargo.toml:71-76`).
  CI fährt Tests damit: `cargo test --profile ci --workspace --all-targets`
  (`ci.yml:59`, `TEST-RIG.md:28-30`).

RAM-Schutz für alle Worktrees darunter: `.cargo/config.toml`
(`.cargo/config.toml`):

```toml
[build]
jobs = 4
```

(RUNBOOK §3.1: deckelt Parallelität als RAM-Schutz.)

Standardbefehle (`INSTALL-RUST.md`, `RELEASE.md` §4):

```sh
cargo build --release -p sail-cli
./target/release/sail-rs --version
./target/release/sail-rs gen_lean --model aarch64_small -o out/
cargo test --workspace
```

Hinweise aus den Quellen, wörtlich beachtet:

- `docs/release-profiles.toml.md` ist **nur ein Vorschlag** (LTO-fat,
  `codegen-units = 1`): Copy-paste-Snippet für den Maintainer, bewusst
  kein Eingriff in die Workspace-`Cargo.toml` — der aktive Stand bleibt
  thin-LTO wie oben.
- `docs/CLI_NEEDED.md` ist ebenfalls **nur spezifiziert, nicht umgesetzt**:
  `--version` soll `sail-rs <version>` (`git describe`, Fallback
  `CARGO_PKG_VERSION`) ausgeben. Implementiert ist: `-v`/`--version`
  setzt `print_version` (`args.rs:100,342-344`), der Help-Text beginnt mit
  `sail-rs {pkg_version} (OCaml-compatible frontend for Sail 0.20.3)`
  (`args.rs:519-521`, `COMPAT_SAIL_VERSION = "0.20.3"` in `args.rs:13`).
- `Cargo.lock` und `target/` werden nie committet (`.gitignore`,
  `RELEASE.md` §6, `INSTALL-RUST.md`).
- Leichte Checks: `./scripts/check.sh` = `cargo fmt --check` + `cargo
  clippy -- -D warnings` + `forbid(unsafe_code)`-Prüfung je Crate-Root +
  unsafe-Token-Grep (`check.sh`, `branch-protection.md`).
- CI-Jobs (`ci.yml`, `branch-protection.md`): `fmt`, `clippy`, `test`
  (`--profile ci`), `e2-sorry-gate` (blockierend), `unsafe-verbot`
  (blockierend: geiger + Token-Grep + `check.sh`), `perf-budget`
  (**warnend**, `continue-on-error`), `msrv`, `deny-audit` (**warnend**,
  `continue-on-error`, bis AUDIT-Aktionen A1–A5 umgesetzt sind).

## 2. Rig-Nutzung (schwere Läufe nur begrenzt in tmux)

Zweck (`TEST-RIG.md`): alle schweren Läufe (`cargo test`/`cargo build`,
`sail`, `lake`) laufen **nicht** in der Chat-Session, sondern entkoppelt
in tmux und speicherbegrenzt via systemd-MemoryMax (E3-OOM-Schutz).

Limits (`TEST-RIG.md`, `limited-run.sh`, `tmux-test.sh`):

| Profil  | MemoryMax | Wofür |
|---------|-----------|-------|
| Builder | `20G` (Default) | `cargo build`, `cargo test`, `sail`-, `lake`-Läufe |
| Checks  | `10G` (`-m 10G`) | `cargo fmt --check`, `clippy`, `scripts/check.sh`, leichte Sonden |

Form: `<Zahl>[M|G]` (z. B. `20G`, `512M`), sonst Exit 2
(`limited-run.sh:76-79`).

Standardbefehle (`TEST-RIG.md` §Profile):

```sh
# Voller Testlauf (Builder-Profil, 20G):
./scripts/tmux-test.sh -s sail-test -w builder -m 20G -- cargo test --profile ci --workspace --all-targets

# Leichte Checks (Checks-Profil, 10G):
./scripts/tmux-test.sh -s sail-test -w checks -m 10G -- ./scripts/check.sh
```

Konventionen (`TEST-RIG.md` §Konventionen, `RUNBOOK.md` §0):

1. **Niemals schwer direkt**: immer `tmux-test.sh` + `limited-run.sh`.
2. **Ein Fenster je Run**: Session = Kampagne, Fenster = einzelner Run;
   wiederholter Aufruf mit gleicher `-s` hängt ein neues Fenster an.
3. **Logging**: Ergebnis nach `logs/limited-run-<timestamp>-<pid>.log`
   (oder `-l DATEI`); Exit-Code unverändert weitergegeben, letzte
   Log-Zeile `limited-run: exit=N`.
4. **`logs/` ist Wegwerfware**: nicht committen (wie `target/`); Rotation
   (`--keep`, Default 10 Dateien, `--max-mb`, Default 50 MiB) löscht alte
   Logs automatisch, Älteste zuerst; das aktive Log wird nie wegrotiert.
5. **E3 beachten**: keine parallelen Schwerläufe aus mehreren Sessions;
   vor neuem Builder-Lauf alte Sessions beenden (`tmux kill-session`).
6. **Skripte ausführbar halten**: `chmod +x scripts/*.sh`.

Warn-Cap/Summary (Lane 92, `TEST-RIG.md` §Log-Hygiene):

| Mittel | Default | Steuerung |
|--------|---------|-----------|
| Warn-Cap (max. N `warning`-Zeilen in Output+Log) | 20 | `-c CAP` / `RIG_WARN_CAP` (`0` = alle unterdrücken) |
| Summary-Mode (identische Folgezeilen kürzen) | aus | `-S` / `--summary` / `RIG_SUMMARY=1` (`--no-summary` schaltet aus) |
| Rotation | 10 Dateien / 50 MiB | `--keep N` / `--max-mb N` / `RIG_KEEP` / `RIG_MAX_MB` (`--max-mb 0` = kein Größen-Deckel) |

tmux-Bedienung (`TEST-RIG.md`, `tmux-test.sh`-Kopf):

```sh
tmux ls                                        # Sessions auflisten
tmux attach -t sail-test                       # anhaengen
# Ctrl-b d                                     # abhaengen (Run laeuft weiter)
tmux capture-pane -p -t sail-test:builder      # Fenster-Inhalt ansehen
tmux kill-window -t sail-test:builder          # einzelnen Run beenden
tmux kill-session -t sail-test                 # ganze Session beenden
```

Voraussetzungen prüfen, lastfrei (`TEST-RIG.md`, Referenz systemd 262 / tmux 3.8):

```sh
systemd-run --user --version
tmux -V
./scripts/limited-run.sh -- true
./scripts/tmux-test.sh --dry-run -- true
```

Technik laut Skript-Köpfen: `limited-run.sh` führt via
`systemd-run --user --scope -p MemoryMax=<MEM>` aus, filtert durch
Warn-Cap/Summary vor `tee` (Cap gilt für Terminal **und** Log) und
prunt beim Start (`tmux-test.sh` prunt zusätzlich vor Fenster-Start).
Neue tmux-Sessions starten in einer eigenen oneshot-Unit
(`remain-after-exit`, `MemoryMax=24G` fürs Rig gesamt;
`tmux-test.sh:151-166`).

`sail-rs`-CLI (Drop-in, Lane 38; Flags aus `args.rs:help_text`,
Treiber-Skizze in `main.rs`/`e2e.rs`):

```sh
sail-rs --lean --lean-output-dir lean -o armv9 -- <dateien.sail>   # Lean-Pfad
sail-rs --just-check -- <dateien.sail>                            # nur bis Typcheck
sail-rs --dry-run -- <args>        # Plan drucken, nichts lesen/schreiben
                                   # (auch via SAIL_RS_DRY_RUN=1, main.rs:307-310)
```

Kompatibilität (`args.rs:563-566`): Single-/Double-Dash sowie `-`/`_`
äquivalent, `--flag=wert` ≡ `--flag wert`; `-`/`--`-Präfix äquivalent
(`RUNBOOK.md` §3.3). Exit-Codes (`args.rs:580-584`): 0 Erfolg
(inkl. `--help`, `-v`, `--dry-run`), 1 Usage-/Sail-Fehler, 2 I/O-fatal.
`sail-rs`-Erweiterungen: `--memo-z3-max-entries` (Default 10000),
`--memo-z3-max-bytes` (Default 67108864) — sail-rs bleibt immer bounded,
OCaml memoisiert unbegrenzt (`args.rs:568-573`).

## 3. Lanes-Arbeitsmodell

Anmerkung zum Spec-Stichwort „Lanes/ASK-Modell": der Begriff `ASK`
kommt im Repo nirgends vor (per `grep` geprüft); beschrieben ist hier
das belegbare Lane-Arbeitsmodell aus Spec-Dateien, `README.md`,
`ACTIVE.md`, `branch-protection.md` und `lane-rotate.sh` — nichts darüber
hinaus.

- **Eine Lane = ein Branch + ein Worktree** (`README.md:3-4`,
  `branch-protection.md:3,36-38`): Branch `lane/<name>`, Worktree
  `sail-rs/worktrees/lane-NN`. Nach Merge: `git worktree remove` +
  Branch löschen.
- **Spec-Format** (`lanes/*.md`, z. B. `lanes/51-test-rig.md`,
  `lanes/38-cli-frontend.md`): Kopf `Branch:` + `Status:`, Abschnitte
  `Ziel` / `Aufgaben` / `Done`.
- **Statuswerte** (in `lanes/*.md` und `ACTIVE.md` belegt): `AKTIV`
  (in Arbeit), `QUEUED` (wartet, u. a. wegen Max-15-Limit), `DONE`
  (gemergt, Worktree+Branch gelöscht), `BLOCKED` (Gate, z. B. Lane 41–44
  bis Port-durch+läuft+RAM-opt, `WAVE3.md` §2).
- **Rotation** (`ACTIVE.md:235-240`, `lane-rotate.sh`): max. 15
  Verzeichnisse in `worktrees/` (Skript bricht bei mehr ab); fertig →
  mergen → Worktree löschen → nächsten Queued-Branch aktivieren:
  `./scripts/lane-rotate.sh <fertig-nr> <neu-nr>`.
- **Merges nur per Pull Request** auf `main`, kein direkter Push, kein
  `--no-verify` (`branch-protection.md`). Merge-Commits mit `-s`
  (Sign-off) und `Co-authored-by: muse-spark-1.3
  <muse-spark-1.3@noreply.local>`-Trailer (`.gitmessage`,
  `rewrite-history-sign.sh:8-10`, `ACTIVE.md`-Merge-Notizen).
- **Merge-Voraussetzungen** (`branch-protection.md`): Lane-13-Report bei
  Codegen/Printer/Rewrites-Berührung (Pflicht, ohne Report kein Merge);
  Lane-14-Budget-Check (Modell-Limits `aarch64_small` <4 GiB,
  `arm-v8.5-a` <16 GiB, `arm-v9.4-a` <24 GiB — bei Überschreitung Shard
  verkleinern oder Pass in Lane 07/09 zurückweisen).

## 4. E1–E4-Kurzfassung (Quelle: `DECISIONS.md`, gekürzt, Sinn unverändert)

- **E1 — Lean ist Master fürs Speichermodell** (mit Beweisen:
  `lean-sail`, `ArmExtras.lean`, generiertes ARM-Modell); Rust
  (`sail-archsem` u. a.) ist nur Spiegel. Sync via Differentialtests
  (Lane 13 + Lane 29); Drift = roter Test = Merge-Blocker.
  Änderungsfluss immer zuerst Lean (+Beweis), dann Spiegel + Difftest.
- **E2 — Nirgends `sorry`**: weder generierter Code noch Templates noch
  Support-Lib; lieber harter Fehler als stilles `sorry`. CI-Gate
  (`e2-sorry-gate`): `grep` über Generat muss leer sein.
- **E3 — Test-Moratorium** (OOM-Schutz, Maschine ohne Swap): Lane-Agents
  führen keine Builds/Tests/`sail`-/`lake`-Läufe aus; Verifikation nur per
  Review + zentral durch den Integrator mit Memory-Limits. „Tests grün"
  als Done-Kriterium ist ausgesetzt.
- **E4 — Out-of-core**: nur Heißes in RAM (aktueller Shard-AST, aktiver
  Rewrite-State, heiße Memo-Einträge); Warm per mmap (fertige Shards,
  Prelude/Builtins); Kalt in Disk-Cache mit Content-Hash-Schlüssel
  (Memo-Spills, inkrementelle Artefakte). Voraussetzungen: stabile IDs per
  Content-Hash (Lane 37), explizite Phasen-Barrieren; Cross-Shard nur als
  Summaries in RAM. Swap bleibt Airbag, ersetzt die Architektur nicht.
