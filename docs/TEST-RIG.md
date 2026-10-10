# TEST-RIG (Lane 51): schwere Läufe nur begrenzt in tmux

Zweck: Alle schweren Läufe (`cargo test`/`cargo build`, `sail`, `lake`)
laufen **nicht** in der Chat-Session, sondern entkoppelt in tmux und
speicherbegrenzt via systemd-MemoryMax. Hintergrund: E3-Test-Moratorium
(OOM-Schutz).

## Limits

| Profil  | MemoryMax | Wofür |
|---------|-----------|-------|
| Builder | `20G` (Default in `limited-run.sh`) | `cargo build`, `cargo test`, `sail`, `lake`-Läufe |
| Checks  | `10G` (`-m 10G`) | `cargo fmt --check`, `clippy`, `scripts/check.sh`, leichte Sonden |

Die Form muss `<Zahl>[M|G]` sein (z.B. `20G`, `512M`), sonst bricht
`limited-run.sh` mit Exit 2 ab.

## Profile / Standardbefehle

```sh
# Voller Testlauf (Builder-Profil, 20G):
./scripts/tmux-test.sh -s sail-test -w builder -m 20G -- cargo test --profile ci --workspace --all-targets

# Leichte Checks (Checks-Profil, 10G):
./scripts/tmux-test.sh -s sail-test -w checks -m 10G -- ./scripts/check.sh
```

`--profile ci` ist im Workspace definiert (`[profile.ci]`, erbt von `release`);
damit laufen CI-nahe Einstellungen lokal reproduzierbar. Solange einzelne
Lanes kein eigenes Profil brauchen, bleibt `ci` der einzige Custom-Name.

## Konventionen

1. **Niemals schwer direkt**: kein `cargo test`/`cargo build`/`lake build`
   im Vordergrund der Chat-Session — immer `tmux-test.sh` + `limited-run.sh`.
2. **Ein Fenster je Run**: Session = Kampagne (z.B. `sail-test`), Fenster =
   einzelner Run (z.B. `builder`, `checks`, `lake-e2e`). Wiederholter Aufruf
   mit gleicher `-s` hängt ein neues Fenster an statt die Session zu ersetzen.
3. **Logging**: `limited-run.sh` filtert Output erst durch Warn-Cap/Summary
   (Lane 92) und spiegelt das Ergebnis nach
   `logs/limited-run-<timestamp>-<pid>.log` (oder `-l DATEI`); der Exit-Code
   des Befehls wird unverändert weitergegeben und steht auch als letzte
   Log-Zeile (`limited-run: exit=N`). Kurzläufer (z.B. `true`) schließen
   ihr tmux-Fenster sofort wieder — das Ergebnis steht dann nur im Log.
   Der Cap gilt für Terminal **und** Log: volle Warning-Fluten landen
   nirgends mehr (keine 33MB-Logs).
4. **`logs/` ist Wegwerfware**: nicht committen (gilt wie `target/`).
   Rotation (`--keep`, Default 10 Dateien, `--max-mb`, Default 50 MiB)
   löscht alte `limited-run-*.log` automatisch (Älteste zuerst).
5. **E3 beachten**: keine parallelen Schwerläufe aus mehreren Sessions;
   vor neuem Builder-Lauf alte Sessions beenden (`tmux kill-session`).
6. **Skripte ausführbar halten**: `chmod +x scripts/*.sh`.

## tmux-Bedienung (Reattach-Doku)

```sh
tmux ls                                        # Sessions auflisten
tmux attach -t sail-test                       # anhaengen
# Ctrl-b d                                     # abhaengen (Run laeuft weiter)
tmux capture-pane -p -t sail-test:builder      # Fenster-Inhalt ansehen
tmux kill-window -t sail-test:builder          # einzelnen Run beenden
tmux kill-session -t sail-test                 # ganze Session beenden
```

`tmux-test.sh --dry-run` zeigt nur den tmux-Aufruf, ohne etwas zu starten —
nützlich zur Kontrolle vor schweren Läufen.

## Log-Hygiene (Lane 92, gegen 33MB-Logs)

Warning-Fluten (z.B. wiederholte Compiler-Warnings) haben früher volle
`logs/`-Dateien geschrieben. Jetzt gilt:

| Mittel | Was | Default | Steuerung |
|--------|-----|---------|-----------|
| Warn-Cap | max. N Zeilen mit `warning` (case-insensitiv) in Output+Log; Rest wird gezählt und als eine `limited-run: … Warning-Zeilen unterdrueckt`-Zeile gemeldet | 20 | `-c CAP` / `RIG_WARN_CAP` (`0` = alle unterdrücken) |
| Summary-Mode | aufeinanderfolgende identische Zeilen kürzen (erste 2 gezeigt, Rest gezählt) | aus | `-S` / `--summary` / `RIG_SUMMARY=1` (`--no-summary` schaltet aus) |
| Rotation | neueste N `limited-run-*.log` behalten + Gesamtgröße deckeln (Älteste zuerst) | 10 Dateien / 50 MiB | `--keep N` / `--max-mb N` / `RIG_KEEP` / `RIG_MAX_MB` (`--max-mb 0` = kein Größen-Deckel) |

`tmux-test.sh` gibt Caps/Rotation an `limited-run.sh` weiter und prunt alte
Logs schon vor dem Fenster-Start; `limited-run.sh` prunt beim Start zusätzlich
selbst (gilt auch für Direktaufrufe). Das aktive Log wird nie wegrotiert.

```sh
# Standard (Caps an, Rotation an):
./scripts/tmux-test.sh -s sail-test -w checks -m 10G -- ./scripts/check.sh

# Gesproechiger (mehr Warnings sehen):
./scripts/tmux-test.sh -s sail-test -w builder -m 20G -c 100 -- cargo test --profile ci --workspace

# Summary-Mode an:
./scripts/tmux-test.sh -s sail-test -w builder -m 20G -S -- cargo test --profile ci --workspace
```

## Voraussetzungen prüfen (keine Last)

```sh
systemd-run --user --version   # systemd user scope verfügbar?
tmux -V                            # tmux installiert?
./scripts/limited-run.sh -- true   # Smoke: muss exit=0 melden
./scripts/tmux-test.sh --dry-run -- true   # Smoke: zeigt nur tmux-Zeile
```

Referenz-Umgebung (Lane-51-Abnahme): systemd 262, tmux 3.8.

## Dateien

- `scripts/limited-run.sh` — Wrapper `systemd-run --user --scope
  -p MemoryMax=<N>G`, Warn-Cap/Summary-Filter (Lane 92), Logging,
  Exit-Code-Weitergabe, Log-Rotation.
- `scripts/tmux-test.sh` — Session-/Fenster-Layout, ein Fenster je Run,
  Cap-/Rotations-Weitergabe an `limited-run.sh` plus Pre-Prune (Lane 92).
- `lanes/51-test-rig.md` — Lane-Spec.
