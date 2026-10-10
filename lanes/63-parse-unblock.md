# Lane 63 — parse-unblock (Lane-61-Follow-up)
Branch: `lane/63-parse-unblock` | Status: AKTIV

## Ziel
Lane-61-Blockade #4–#7 + #42 auflösen: parse-Defs (Val/Fundef/Mapdef/Scattered) ohne
Lane-02-Vollgrammatik auf sail-ast bringen (Lexer + Grammatik-Skelett lesen).

## Aufgaben
- Minimalinvasive Brücke (kein Grammatik-Rewrite), Tests als Code (NICHT ausführen, E3).
- `#![forbid(unsafe_code)]`.

## Done
- parse-Defs nutzen sail-ast, NEEDED-Einträge geschlossen.

## Umsetzung (Lane 63, E3: kein Build/Test-Run)
- `crates/sail-parse/src/bridge.rs`: `bridge_file`/`bridge_parsed` → `sail_ast::Ast`
  + `SimpleInterner` + `unparsed`-Sidecar (Rohtext per `NodeId`) + `deferred`
  (verlustfreie `mini_ast`-Reste). Kein Grammatik-Rewrite, `#![forbid(unsafe_code)]`
  via Workspace-Lints.
- Gebrückt (#4–#7): `Val` (Typ `Unknown`), `Fundef` (Skelett-`Funcl` pro Knoten,
  `rec`-Flag aus `sail-lex`), `Mapdef` (leere Klauseln), `Scattered`
  (Function/Funcl/Mapping/Mapcl/Enum/Variant/End; bare `clause` per `<->`-Heuristik).
  Dazu #42-Meta: `Impl`, `Fixity`, `Overload`, `Default(Order)`, `Pragma`
  (inkl. `file_start`/`file_end`-Marker). Offen: Type/Let/Constraint/Register/
  Measure/Outcome/Instantiation/Other + alle Exp/Typ-Inhalte (Lane 02/17).
- Namen/Dispatch aus `sail-lex`-Tokenarten (Branch-Stand), Klausel-Formen nach
  `sail-grammar/src/ast.rs` gelesen (keine Dependency, dort Skelett-Status).
- Tests als Code: `crates/sail-parse/tests/bridge_ast.rs` (NICHT ausgeführt).
- `.gitignore`: `target/` + `Cargo.lock` bereits enthalten, unverändert.
