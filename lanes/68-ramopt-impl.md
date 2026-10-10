# Lane 68 — ramopt-impl (RAM runter im Code)
Branch: `lane/68-ramopt-impl` | Status: DONE (Code steht, Rig-Verifikation offen)

## Ziel
Größte RAM-Hebel aus RAM_ANALYSIS.md (Lane 58, gemergt) umsetzen — auf vereinheitlichtem
AST (Lane 61, gemergt): String-Interning überall (statt String-pro-Vorkommen),
doppelten Interner-Speicher vereinen, subst ohne Neu-Boxen pro Knoten, env.clone() vermeiden.

## Aufgaben
- Top-3-Hotspots zuerst (Duplikate→Unify ist drin; hier: Interning + Boxen + Clones).
- Verhalten identisch, Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Umgesetzt (E3: kein Build/Test ausgeführt, Verifikation via Test-Rig später)
1. **Interner-Speicher vereint** (`sail-ast/src/interner.rs`): `SimpleInterner`
   speichert jeden String nur 1× als `Arc<str>`, geteilt zwischen Map-Key und
   Vec-Eintrag (vorher 2× `to_owned`). Symbole, `resolve`, JSON-Format, `Send+Sync`
   unverändert; serde `rc`-Feature in `sail-ast/Cargo.toml` dafür aktiviert.
2. **subst ohne Neu-Boxen** (`sail-nexp/src/expr.rs`): neue `subst_cow`-Methoden
   für `NExp`/`NConstraint` (unveränderte Teilbäume → `Borrowed`, null Allokation);
   `subst` delegiert daran (werte-identisch, inkl. Bool-Var-Faltung).
3. **Clone-Diät** (`sail-resolve/src/env.rs`: 5 Inserts per `entry`-API, Wert wird
   bewegt statt geklont, Fehlerinhalte identisch; `sail-ty/src/env.rs`: `TypeEnv`
   teilt val/register-Tabellen per `Arc` + CoW (`env.clone()` pro `let` ist nur noch
   Refcount-Bump + kleine Locals-Kopie); `sail-ty/src/check.rs`: sinnloser
   `inner_body`-Clone entfernt; `sail-ty/src/quant.rs`: `bind` per Entry (1 Lookup),
   `freshen` mit `with_capacity`).

## Bewusst nicht in dieser Lane
- `TokenKind`-/`LitKind`-/`Kid`-String-Payloads bleiben `String`: Umstellung auf
  `Symbol` bräche Lexer-Snapshot-Tests und Parser-Verträge — eigener Lane mit Rig.
- `simplify`/`internal_lets` behalten Owned-Rewrites (Semantik-Erhalt ohne Compiler
  nicht absicherbar); `subst_cow` ist der wiederverwendbare Mechanismus dafür.

## Done
- Hotspots umgesetzt, Verifikation via Test-Rig (später).
