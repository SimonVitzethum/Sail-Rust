# Lane 51 — test-rig (systemd+tmux)
Branch: `lane/51-test-rig` | Status: AKTIV

## Ziel
Test-Rig für die Haupt-Session: alle schweren Läufe (cargo test/build, sail, lake) nur unter systemd-MemoryMax in tmux, unabhängig von der Chat-Session.

## Aufgaben
- `scripts/limited-run.sh`: Wrapper `systemd-run --user --scope -p MemoryMax=<N>G <cmd>`, Logging nach Datei, Exit-Code-Weitergabe.
- `scripts/tmux-test.sh`: tmux-Session-Layout (ein Fenster je Run), Reattach-Doku.
- `docs/TEST-RIG.md`: Limits (Default 20G Builder, 10G Checks), Konventionen.
- Nur Skripte/Doku, KEINE schweren Läufe selbst starten (max. `true`-Smoke).

## Done
- Rig vorhanden + dokumentiert, Smoke ok.
