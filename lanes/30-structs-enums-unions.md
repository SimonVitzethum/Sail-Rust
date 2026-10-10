# Lane 30 — structs-enums-unions
Branch: `lane/30-structs` | Status: QUEUED

## Ziel
`struct/enum/variant/record/abbrev/bitfield`-Typdefs.

## Aufgaben
- `doc_typdef`-Port inkl. `deriving Inhabited/BEq/Repr`, `open`-Emits, `fp_bits`-Skip dokumentieren.
- `TD_abstract/TD_bitfield` = klarer Fehler statt Panic.

## Done
- `enum.sail`, `bitfield.sail`, `option.sail`-Tests grün.
