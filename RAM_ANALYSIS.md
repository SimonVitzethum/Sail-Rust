# RAM-Hotspot-Analyse (Lane 58, Analyse-only — kein Code-Umbau)

Stand: Worktree `lane/58-ram-opt`, Basis-Merge `b079704`. Methode: reine
Code-Lesung + `grep`/`find`-Zählung über `crates/` (182 `.rs`-Dateien,
~39.8 kLOC). Keine Builds, keine Tests, keine Profiling-Läufe (E3-Gate).
Alle Zahlen sind statische Fundstellen, keine RSS-Messungen.

## 1. Größte Dateien (Top 15 nach Bytes)

| Rang | Datei | Größe | LOC |
| --- | --- | --- | --- |
| 1 | `crates/sail-lex/src/lexer.rs` | 39.8 KB | 1175 |
| 2 | `crates/sail-sve-sme/src/parser.rs` | 39.0 KB | 1052 |
| 3 | `crates/sail-parse/src/parser.rs` | 39.0 KB | 1052 |
| 4 | `crates/sail-sve-sme/src/patterns.rs` | 31.3 KB | 907 |
| 5 | `crates/sail-lake/src/lib.rs` | 29.5 KB | 809 |
| 6 | `crates/sail-cli/src/args.rs` | 28.9 KB | 781 |
| 7 | `crates/sail-grammar/src/lexer.rs` | 25.7 KB | 850 |
| 8 | `crates/sail-oracle/src/oracle.rs` | 24.6 KB | 698 |
| 9 | `crates/sail-diag/src/code.rs` | 23.7 KB | 541 |
| 10 | `crates/sail-lean/src/lib.rs` | 21.7 KB | 655 |
| 11 | `crates/sail-extern/src/lib.rs` | 21.6 KB | 627 |
| 12 | `crates/sail-regs/src/lib.rs` | 19.0 KB | 555 |
| 13 | `crates/sail-cache/src/lib.rs` | 18.3 KB | 529 |
| 14 | `crates/sail-nexp/src/simplify.rs` | 16.9 KB | 506 |
| 15 | `crates/sail-patterns/src/lib.rs` | 16.6 KB | 474 |

Größte Crates (gesamt): `sail-prelude` 232 KB (enthält `sail-lib/`
Vendored-Sail-Quellen, kein RZ-Opt-Ziel), `sail-sve-sme` 160 KB,
`sail-grammar` 136 KB, `sail-parse` 128 KB, `sail-lex` 108 KB,
`sail-ty` 100 KB.

## 2. Klone / Duplikate (exakt nachgewiesen per md5)

Byte-identische Dateien zwischen `sail-parse` und `sail-sve-sme`:

- `src/lexer.rs` — md5 `478033fc…` identisch (10.0 KB)
- `src/span.rs` — md5 `46f58beb…` identisch (2.4 KB)
- `src/mini_ast.rs` — md5 `da85ac59…` identisch (5.1 KB)
- `src/parser.rs` — **fast** identisch: `diff` = 4 Zeilen (nur 1
  Doc-Kommentar: `parse_files_parallel` vs `patterns::analyze_files_parallel`)

Wirkung: doppelter Compile-Aufwand, doppelte `.rmeta`/Codegen-Kosten,
doppelte Wartung. Kein weiterer md5-Dupe im Workspace gefunden.
Beinahe-Dupes (thematisch, nicht byte-identisch): `sail-lex/src/lexer.rs`
vs `sail-grammar/src/lexer.rs` (getrennte Lexer für Sail2 vs Grammatik-Port),
`sail-mapping/src/patterns.rs` vs `sail-sve-sme/src/patterns.rs` (differieren).

## 3. Vec-/String-Allokationsmuster (workspace-weit, statisch)

| Muster | Treffer |
| --- | --- |
| `to_string()` | 694 |
| `to_owned()` | 305 |
| `.clone()` | 348 |
| `format!` | 364 |
| `.push(` | 326 |
| `Box::new` | 391 |
| `.collect()` + `collect::<` | 219 |
| `Vec::new` | 103 vs `with_capacity` 32 (~3:1 ohne Reservierung) |
| `String::new` | 53, `String::from` 33 |
| `HashMap<` | 24, `BTreeMap` | 54 |

Pro-Crate-Spitzen:

- `.clone()`: `sail-grammar` 114 (fast alle in `sail.lalrpop`-Aktionen,
  z. B. `lo.clone()`, `i.clone()`, `a.clone()` in Feld-/Pattern-Regeln),
  `sail-nexp` 41, `sail-mapping` 34, `sail-resolve` 31, `sail-ty` 27,
  `sail-lower` 25.
- `to_string/to_owned`: `sail-grammar` 128, `sail-mapping` 92,
  `sail-loops` 88 (Großteil Test-Fixtures mit `"…" .to_owned()`-Blöcken in
  `render.rs:280ff`, aber auch Render-Pfade), `sail-rewrite` 79,
  `sail-sve-sme` 61, `sail-lex` 52, `sail-lower` 51.
- `format!`: `sail-bitvec` 44 (Fehlertexte + `render`-Pfad in
  `bitfield.rs`/`lit.rs`, z. B. `format!("0x{digits}#{width}")`),
  `sail-oracle` 38 (Fehlerkontext, Scratch-Pfade), `sail-lake` 36
  (Manifest-/Import-Rendering, `String::new` + `push_str(format!(…))`-Ketten).
- `Box::new`: `sail-nexp` 176, `sail-grammar` 64, `sail-lower` 50,
  `sail-ty` 47 — alle vier sind rekursive AST-Rewriter (`subst`,
  `simplify`, `internal_lets`, `remove_blocks`), die pro Knoten neu boxen.
- `collect()`: `sail-mapping` 20, `sail-oracle` 19, `sail-lower` 16,
  `sail-parse` 13.

Strukturelle String-Last:

- `sail-lex/src/token.rs`: `TokenKind` trägt ~13 `String`-Payload-Varianten
  (`Id`, `TyVar`, `Op`, `Num`, `Hex`, `Bin`, `Real`, `Str`, …) — jeder Token
  allokiert.
- `sail-ast/src/ast.rs`: Literale als `String` (`Num/Hex/Bin/String/Real`,
  `Constant(String)`, `Set{…members: Vec<String>}`), `bindings:
  Vec<(String, String)>`.
- `sail-nexp/src/expr.rs`: `Kid = String`, `NExp::Id(String)`,
  `Var(Kid)`, `App(String, Vec<NExp>)` — Kind-Variablen und Funktionsnamen
  je Vorkommen neu allokiert; `subst()` (`expr.rs:134ff`) baut den gesamten
  Baum mit frischen `Box`en neu auf.
- `sail-ast/src/interner.rs`: `SimpleInterner` speichert jeden String
  **doppelt** (`HashMap<String, Symbol>` + `Vec<String>`, `intern()` ruft
  `text.to_owned()` zweimal auf) — 2× Speicher pro Eintrag plus
  doppelter Hash-Schlüssel.
- `sail-resolve/src/env.rs:191ff`: 5 `HashMap<String, …>`-Environments;
  jede `insert`-Stelle klont Schlüssel **und** Wert
  (`e.name.clone(), e.clone()`).
- `sail-ty/src/check.rs`: `env.clone()` pro `let`-Bindung (`:215`),
  `scheme.clone()`/`name.to_string()` pro Item (`:118ff`).
- `sail-lake/src/lib.rs:360ff`: O(n²)-Dedup via `seen.iter().any(…)` +
  `push` in `Vec::new()` ohne Kapazität.

## 4. Monomorphisierungs-Risiken (niedrig insgesamt)

- Generik-Signale: `<T` 111, `impl<…>` 31, `struct …<` 31, `enum …<` 3,
  `trait …<` **0**, `#[derive]` 393 (davon `Clone` 1× explizit gezählt —
  Zählung hier unvollständig, da mehrzeilige Derives nicht alle erfasst;
  `Debug` 230 als Proxy für große AST-Derives).
- Einziges nennenswertes generisches Laufzeit-Gerüst:
  `ShardArena<T>` (`sail-arena/src/shard.rs`) — pro `T`-Instanz eine
  Codegen-Kopie; aktuell wenige Instanzen, Risiko erst bei breiter
  Parametrisierung über viele AST-Typen.
- Kein generischer Trait-Overhead, keine `Rc`/`Arc`-Verbreitung
  (`Rc::new` 1, `Arc::new` 9). Kein `dyn`-Heavy-Design gefunden.
- Codegen-Blähung kommt eher aus Duplikat-Crates (Abschnitt 2) und
  großen Enums mit vielen `String`-Varianten als aus Generics.

## 5. Priorisierte Opt-Liste für den späteren Opt-Pass (nach Unify)

P0 — zuerst (größter Hebel, geringes Risiko):

1. `sail-parse`/`sail-sve-sme`-Dupe zusammenführen (3 byte-identische
   Dateien + 1 Fast-Dupe in ein Shared-Crate auslagern). Spart Compile- und
   Binary-Größe, Vorbedingung für alles Weitere in diesen Crates.
2. `SimpleInterner` (`sail-ast/src/interner.rs`) Einmal-Speicherung:
   `Vec<String>` als Owner + `HashMap<&str, Symbol>`-View bzw. `HashSet` —
   halbiert Interner-Speicher, ein `to_owned` pro `intern()` weniger.
3. Token-/Literal-Payloads internen: `TokenKind`-`String`s und
   `NExp::Id/Var/App`-Namen auf `Symbol`/`&str`-Arena umstellen
   (`sail-lex/src/token.rs`, `sail-nexp/src/expr.rs`). Beseitigt die
   breiteste pro-Vorkommen-Allokation im Frontend.

P1 — danach (mittlerer Hebel):

4. `subst`/`simplify`/`internal_lets` ohne Neu-Boxen bei Unverändertheit:
   Cow-/Early-return-Muster in `sail-nexp/src/expr.rs`,
   `sail-nexp/src/simplify.rs`, `sail-lower/src/anf.rs` (heute
   `Box::new` pro Knoten, 176+50 Fundstellen).
5. `sail-resolve/src/env.rs`: `entry`-API + `Rc`/`Arc`-geteile Einträge
   statt `name.clone(), entry.clone()` pro Insert.
6. `Vec::new` → `with_capacity` / `collect` mit `size_hint` an den
   ~3:1-unreservierten Stellen (`sail-regs:101`, `sail-lake:360`,
   `sail-sve-sme/parser.rs:569`, `sail-cache:214/341`).
7. `sail-bitvec`/`sail-oracle`/`sail-lake`: `format!` nur im Fehlerpfad
   lassen; Render-Pfade (`bitfield.rs`, `lit.rs:146ff`, `lake/lib.rs:244ff`)
   auf `write!` in vorreservierten `String` umstellen.

P2 — Nice-to-have (kleiner Hebel / erst mit Messung):

8. `sail-grammar/sail.lalrpop`-Aktions-Clones (`lo.clone()`, `i.clone()`,
   `a.clone()`) nach Grammatik-Unify per Borrow-/Move-Analyse reduzieren.
9. `sail-lake/src/lib.rs:360ff` O(n²)-Dedup durch `HashSet` ersetzen.
10. `sail-ty/src/check.rs:215` `env.clone()` pro Scope durch persistente/
    geschachtelte Environments ersetzen (erst nach Unify der Ty-Umgebungen).
11. `ShardArena<T>`-Instanzen zählen und bei Bedarf pro `T` Shard-Größen
    deckeln (`sail-arena/src/sizing.rs`), statt generisch zu duplizieren.

Explizit **nicht** Teil dieses Reports: Code-Umbau, Messwerte, Unify selbst.
Alle Opts sind für den Opt-Pass nach dem Unify vorgesehen.
