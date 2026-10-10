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
3. **Logging**: `limited-run.sh` spiegelt alles nach
   `logs/limited-run-<timestamp>-<pid>.log` (oder `-l DATEI`); der Exit-Code
   des Befehls wird unverändert weitergegeben und steht auch als letzte
   Log-Zeile (`limited-run: exit=N`). Kurzläufer (z.B. `true`) schließen
   ihr tmux-Fenster sofort wieder — das Ergebnis steht dann nur im Log.
4. **`logs/` ist Wegwerfware**: nicht committen (gilt wie `target/`).
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
  -p MemoryMax=<N>G`, Logging, Exit-Code-Weitergabe.
- `scripts/tmux-test.sh` — Session-/Fenster-Layout, ein Fenster je Run.
- `lanes/51-test-rig.md` — Lane-Spec.
