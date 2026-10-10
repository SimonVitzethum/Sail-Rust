# Payload-Intern-Vertrag (Lane 71)

Schließt die Lane-68-Abgrenzung (`lanes/68-ramopt-impl.md`, „Bewusst nicht in
dieser Lane“): `TokenKind`- / `LitKind`- / `Kid`-Payloads sind keine `String`s
mehr, sondern internierte `Symbol`s. Jede distinkte Zeichenkette steht genau
einmal in der Interner-Tabelle statt einmal pro Vorkommen.

E3-Status: Tests als Code, **nicht ausgeführt** (`docs/TEST-RIG.md`). Die
Difftest-Liste unten (§4) ist der Abnahmeauftrag fürs Rig.

## 1. Was sich geändert hat

| Ort | Vorher | Nachher |
|---|---|---|
| `sail-lex/src/token.rs`: `TokenKind::{Id,TyVar,Op,Num,Hex,Bin,Real,Str,MultiLineStr,Attribute,StructuredPragma,DocBlock,DocLine}` + `Pragma{name,value}` + `Fixity{op}` | `String` | `sail_ast::Symbol` |
| `sail-lex/src/token.rs`: `TokenKind::Error` | `String` | `String` (**Ausnahme**, s. §2) |
| `sail-lex/src/lexer.rs`: `lex(file, src)` | 2 args | `lex(file, src, interner: &mut impl Interner)` |
| `sail-ast/src/ast.rs`: `LitKind::{Num,Hex,Bin,String,Real}` | `String` | `Symbol` (`LitKind` ist neu auch `Copy`) |
| `sail-ast/src/interner.rs`: `Symbol` | `… PartialEq, Eq, Hash …` | zusätzlich `PartialOrd, Ord` (für `BTreeMap<Kid, _>`) |
| `sail-nexp/src/expr.rs`: `Kid` | `type Kid = String` | `type Kid = sail_ast::Symbol` (neue `sail-ast`-Dep) |
| `sail-nexp`: `NExp::var`, `Facts::{with_lower,with_upper,with_nonzero,lower,upper,is_nonzero_var}` | `impl Into<String>` / `&str` | `Kid` |

Mitgezogen (mechanisch, verhaltensidentisch): `sail-parse`‑Bridge
(`lex_kinds`/`op_by_lex`/`has_rec`/`name_word` nehmen den Interner),
`pat_to_ast`/`pat_from_ast` (`sail-patterns`, `sail-mapping`),
`lit_from_ast` (`sail-bitvec`), `miniexp_to_ast`/`miniexp_from_ast`
(`sail-ty`), `render_sail_lit` (`sail-regs`, neu mit Interner-Parameter).

## 2. Parser-Vertrag (für Lane 17 / `sail-parse`)

1. **Gleicher Interner empfohlen:** `lex` in den AST-Interner schreiben
   lassen (`bridge.rs` nutzt den Bridge-Interner direkt); dann sind
   `Symbol`s ohne Umtragen auflösbar.
2. **Vergleiche immer über `resolve`:** `TokenKind::Id(s)` prüfen als
   `interner.resolve(s) == Some("rec")`, nie als String-`==`.
   `kw_of` (Unit-Varianten) ist unverändert.
3. **`Error` bleibt `String`:** Recovery-Texte duplizieren
   `LexError::message` und dürfen die Tabelle nicht fluten. `matches!(…,
   TokenKind::Error(_))`‑Filter sind unverändert gültig.
4. **`kind_name()` ist unverändert** (kein Payload drin); `is_doc()`,
   `is_eof()`, `code_tokens()`, Spans und Fehlerpolitik sind unverändert.
5. **Threads:** ein Interner pro Lex-Aufruf/Thread (siehe
   `tests/lean_corpus.rs`); Symbole sind nur innerhalb ihrer Tabelle gültig.

## 3. Snapshot-Verträge

**Form ändert sich (Rig-Diffs erfordern Abnahme, kein Stillhalten):**

- `crates/sail-lex/tests/unit_snap.rs` — alle Payload-Erwartungen sind
  jetzt `it.intern("…")`-Symbole; neu: `payloads_are_interned_symbols`
  (Dedup + `resolve` + `len`) und `Symbol` im `Send+Sync`-Test.
- `crates/sail-lex/tests/lean_corpus.rs` — `lex` mit Thread-lokalem
  Interner + `Id`-Resolvability-Check.
- AST-JSON mit `LitKind`-Payloads: `{"Num":"42"}` → `{"Num":<u32>}`.
  Symbole sind nur mit der mitausgegebenen Tabelle lesbar (`Name`/`KOpt`
  hatten dieses Format schon — kein neues Prinzip). Neu abgedeckt:
  `json_roundtrip_covers_interned_lit_payloads` (`sail-ast/tests/roundtrip.rs`).
- `sail-nexp`-`Debug`: `Var("n")` → `Var(Symbol(0))` (nur Test-/Logtext).
- `Facts`-Zugriffe nehmen `Kid` statt `&str` (`pred.rs`-Tests als Vorlage).

**Unverändert (Difftest muss hier stillhalten):**

- Token-Inventory, `kind_name()`s, Spans, `LexError`-Texte, `Error`-Payloads.
- `NExpKind::Constant(String)`, `ConstraintKind::Set.members`,
  `Value`, `Pragma`/`Extern`/`Config`/`Fixity.level`, `VectorSubrange`
  (`lo`/`hi`) — bewusst **außerhalb** dieser Lane (s. §5).
- `sail-grammar` (`Tok`/`Lit`, eigenes `String`-Universum) — unberührt.
- `sail-ty`- / `sail-int`-eigene `Nexp`/`NConstraint`-Miniwelten — unberührt
  (Vorab-Duplikate, Ablösung erst mit Grammatik-Vollmerge).

## 4. Difftest-Liste (Rig-Auftrag)

Schwerläufe nur via `scripts/tmux-test.sh` + `limited-run.sh`
(`docs/TEST-RIG.md`, Builder-Profil). Reihenfolge: leicht → schwer.

```sh
# 1. Checks-Profil: fmt + clippy über alles (fängt Clone/Copy- und
#    Import-Fehler aus den mechanischen Edits).
./scripts/tmux-test.sh -s sail-test -w checks -m 10G -- ./scripts/check.sh

# 2. Gezielte Testziele dieser Lane (Builder-Profil):
./scripts/tmux-test.sh -s sail-test -w lane71 -m 20G -- \
  cargo test --profile ci -p sail-ast -p sail-lex -p sail-nexp -p sail-parse \
    -p sail-patterns -p sail-mapping -p sail-bitvec -p sail-ty -p sail-regs

# 3. Voller Workspace (Spätfolge-Prüfung aller mitgezogenen Bridges):
./scripts/tmux-test.sh -s sail-test -w builder -m 20G -- \
  cargo test --profile ci --workspace --all-targets
```

Abnahme-Checkliste pro Ziel:

- [ ] `sail-lex unit_snap`: alle Payload-Vergleiche grün (Symbole aus
      derselben Tabelle); `payloads_are_interned_symbols` belegt Dedup.
- [ ] `sail-lex lean_corpus`: fehlerfrei wie vor Lane 71 (Tokenzahlen
      identisch zu `main`; nur Zählung/Fehler verglichen, keine Payloads).
- [ ] `sail-ast roundtrip`: alte Snapshots ohne `LitKind`-Payloads laden
      unverändert; `json_roundtrip_covers_interned_lit_payloads` grün.
- [ ] `sail-nexp` (unit + `prop`): `kids_are_stable_across_tables` grün;
      `simplify/normalize/provably`-Props ohne neue `proptest`-Regressionen
      (`tests/prop.proptest-regressions` leer wie vorher).
- [ ] `sail-parse bridge_ast`: Operator-/`rec`-/`variant`-Erkennung
      unverändert (löst über `resolve` auf).
- [ ] `sail-regs` bridge-Tests: `0b…`-Init-Rendering identisch
      (`0b0000…` wie String-Pipeline).
- [ ] `cargo test --workspace --all-targets`: keine neuen Failures
      gegenüber Basis-Commit (Liste im Run-Log ablegen).

## 5. Bewusst draußen (Folge-Lane-Kandidaten)

- `NExpKind::Constant(String)` + `ConstraintKind::Set.members:
  Vec<String>` (sail-ast, typ-levele Literale) — eigener Internierungs-
  schnitt, da Konstruktoren ohne Interner-Zugriff gebaut werden.
- `sail-nexp`: `NExp::Id(String)` / `App(String, _)` /
  `NConstraint::Id(String)` (Operator-/Fallback-Namen, nur 3 kanonische
  Werte + Escape-Hatch) — nach `Kid` der nächste Schritt.
- `PatKind::VectorSubrange{lo,hi}`, `Value::{Int,String}`,
  `ExpKind::Config`, `DefKind::{Fixity.level,Pragma}`, `Extern.bindings`.
- `sail-grammar`-Tok-Universum auf `Symbol` heben (mit Grammatik-Skelett
  zusammen).
