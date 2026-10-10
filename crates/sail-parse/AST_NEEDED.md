# AST_NEEDED.md — erwartete `sail-ast`-Schnittstelle (Lane 01)

> Lane 61 status (MIGRATION.md #3–#7): **SPAN CLOSED, SKELETON BRIDGED**.
> `src/span.rs` replaced by `sail_ast::{FileId, Span}` (#3, §6.1c;
> `FileId(0)` per file until the driver-lane file table lands).
> `src/mini_ast.rs` (`Def`/`ParsedFile`/`SpannedDef`/`PragmaArg`,
> #4–#7) is retained: bodies are raw text and only a real
> expression/type grammar (Lane 02) can lower them onto `DefKind`
> with genuine `Exp`/`Typ` nodes; `sail-cli` consumes the skeleton.
> Follow-up owned by Lane 02, seam wired (`sail-ast` dependency).

Stand: Lane 01 (`sail-ast`) ist noch nicht gelandet (Branch `lane/01-ast-core`
ohne Commits). `sail-parse` definiert deshalb lokale Minimal-Typen in
`src/mini_ast.rs`. Dieses Dokument beschreibt, was Lane 01 liefern muss,
damit `sail-parse` migrieren kann.

## Von Lane 01 erwartet (vgl. `lanes/01-ast-core.md`)

- `NodeId` — generationaler Index (`SlotMap`-Stil, nur Indizes, keine Pointer).
- `Span` — Datei + Byte-Range + Zeile/Spalte. Kompatibel zu unserem
  `sail_parse::span::Span` (Felder: `file`, `start_byte`, `end_byte`,
  `start_line`, `start_col`, `end_line`, `end_col`).
- `Interner` — String-Interning (`lasso` o.ä.) für Bezeichner.
- `Ast` — alle Sail2-Knoten (`Typ`, `NExp`, `Constraint`, `Pat`, `Exp`,
  `Def`) als Enums/Structs, `Clone + Send + Sync`, Serde-Snapshots,
  JSON-Roundtrip. Struktur aus `sail/src/lib/extraction/Ast.ml`
  (`DEF_type`, `DEF_constraint`, `DEF_fundef`, `DEF_mapdef`, `DEF_impl`,
  `DEF_let`, `DEF_val`, `DEF_outcome`, `DEF_instantiation`,
  `DEF_fixity`, `DEF_overload`, `DEF_default`, `DEF_scattered`,
  `DEF_measure`, `DEF_loop_measures`, `DEF_register`,
  `DEF_internal_mutrec`, `DEF_pragma`, dazu `DEF_private`/`DEF_attribute`
  Wrapper aus `parser.mly`).

## Was `sail-parse` heute lokal definiert (`src/mini_ast.rs`)

| sail-parse heute | Entspricht in `sail-ast` (erwartet) |
|---|---|
| `Span`, `Spanned<T>`, `ParseError` (`src/span.rs`) | `sail_ast::Span`, Fehler-Typ aus Lane `error-diagnostics` |
| `Def::Val { name, sig_src }` | `DEF_val` (val_spec, voll getypt) |
| `Def::Fundef { name, head_src, body_src }` | `DEF_fundef` (fundef + funcls) |
| `Def::Mapdef`, `Def::Impl` | `DEF_mapdef`, `DEF_impl` |
| `Def::Type { name, body_src }` | `DEF_type` (type_def: abbrev/record/enum/...) |
| `Def::Let`, `Def::Constraint` | `DEF_let`, `DEF_constraint` |
| `Def::Overload { name, ids }` | `DEF_overload` |
| `Def::Fixity { prec, level, name }` | `DEF_fixity` |
| `Def::Default { kind, value }` | `DEF_default` |
| `Def::Scattered { head_src, body_src }` | `DEF_scattered` |
| `Def::Register { name, type_src, init_src }` | `DEF_register` |
| `Def::Measure`, `Def::Outcome`, `Def::Instantiation` | `DEF_measure`/`DEF_loop_measures`, `DEF_outcome`, `DEF_instantiation` |
| `Def::Pragma { name, arg }` (`Line`/`Structured`) | `DEF_pragma` (`Pragma_line`/`Pragma_structured`) |
| `Def::Other { keyword, src }` | entfällt (vollständige Grammatik) |

## Migrationspfad (wenn `sail-ast` landet)

1. `sail-parse/Cargo.toml`: `sail-ast = { path = "../sail-ast" }` als
   Dependency eintragen.
2. `parse_file` gibt `sail_ast::Ast` (bzw. `Vec<Spanned<sail_ast::Def>>`)
   zurück statt `mini_ast::ParsedFile`.
3. Header-Parsing bleibt (Namen, Fixity, Pragma-Args extrahieren), aber
   Bodies werden in echte `Exp`/`Typ`-Knoten geparst statt Rohtext.
4. `file_start`/`file_end`-Wrapping bleibt erhalten (Shard-Marker für
   Lane 11) — entweder als `DEF_pragma`-Knoten wie in OCaml
   (`initial_check.ml::wrap_file`) oder als Metadaten-Anhang, je nach
   Lane-01/11-Abstimmung.
5. `src/mini_ast.rs` wird gelöscht; Tests in `tests/` auf `sail-ast`-Typen
   umstellen. `src/span.rs` wird durch `sail_ast::Span` ersetzt
   (Feldnamen bewusst identisch gewählt).

## Bekannte Skelett-Limits (spätere Lanes)

- Bodies sind Rohtext (keine `Exp`/`Typ`-Grammatik) — Lane 02-Vollausbau.
- Klammerlose Bodies enden am nächsten Definitions-Starter in Spalte 0ff.;
  `let x = e in ...`-Ausdrücke am Zeilenanfang klammerloser Bodies würden
  fälschlich splitten (ARM-Code nutzt fast überall `{...}`-Blöcke).
- Wiederholte `function f`-Klauseln werden je ein `Fundef`-Knoten
  (OCaml fasst `... and ...`-Klauseln in einem `DEF_fundef` zusammen).
- `$[...]`-Attribute bleiben eigene `Pragma`-Knoten statt Anhang an die
  folgende Definition (OCaml: `DEF_attribute`).
- `private`-Präfix wird verworfen (Span bleibt), statt `DEF_private`-Knoten.
