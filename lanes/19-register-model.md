# Lane 19 — register-model
Branch: `lane/19-registers` | Status: QUEUED

## Ziel
Register-Modell (`Register`, `RegisterType`, `RegisterRef`, `readReg/writeReg`).

## Aufgaben
- Register-Defs aus AST extrahieren, `doc_reg_info`-Äquivalent als IR.
- Initialwerte (`reset.sail`), `grouped_regstate`.

## Done
- Registerliste für `aarch64_small` korrekt, Lean-Output-Schnipsel stabil.
