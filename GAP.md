# GAP.md — Lean-Subset-Lücken im ARMv9.4-Modell (Lane 75)

Branch: `lane/75-arm-gap` | Stand: 10.10.2026 | E3 (keine Builds/Läufe)

Korpus (nur gelesen, nicht gebaut):
` /home/simon/Dokumente/Sail/sail-arm/arm-v9.4-a/src` —
37 `.sail`-Dateien, **466.202 Zeilen** (größte: `sysregs_autogen.sail`
160.302, `instrs64_sve.sail` 95.548, `v8_base.sail` 57.139,
`instrs64.sail` 53.106, `instrs32.sail` 48.015).

Methode: `sail-lex`/`sail-parse` (Branch-Stand) als Referenz gelesen
(Token-Inventar `crates/sail-lex/src/token.rs`, Skelett-AST
`crates/sail-parse/src/mini_ast.rs`, Brückenstand
`crates/sail-parse/AST_NEEDED.md`); Häufigkeiten per
read-only Textzählung (`grep -rhoE … | wc -l`, keine Kompilate,
kein `cargo`). Spalten sind `Datei:Zeile`-Belege aus dem Korpus.
Regex-Zählung ≠ Vollparse: exakte Knotenzahlen liefert erst die
Lane-02-Vollgrammatik; Größenordnungen und Owner sind davon
unberührt.

## 1. Lean-Subset (Referenz — was heute abgedeckt ist)

Aus `crates/sail-lean/src/lib.rs` + `conv.rs` (`nexp_from_ast`,
`typ_from_ast`, `typarg_from_ast`):

- **NExp:** `Const Var Sum Minus Times Neg Pow2(2^·)` — **nicht:**
  `If Id App` (konditionale Größen, Funktions-Applikation in Sizes).
- **Typ:** `Unit Int Nat Bool String Bit BitVec Option List Result
  Tuple Vector Var Named` — **nicht:** `Fn Bidir Exist Unknown`.
- **TypArg:** `Typ NExp` — **nicht:** `Bool` (Kind-`Bool`-Argumente).
- **Lit:** `Unit True False Num Hex Bin Str` — **nicht:** `Real`.
- **Effekte/Sigs:** `Pure Stateful EarlyReturn`-Header + Typ-Druck;
  Rümpfe sind Lane-10-`None` (harter `Unsupported`-Fehler, nie `sorry`).
- Abgedeckt und druckvoll im Korpus: `0x`-Literale (16.327),
  `0b`-Literale (69.766), `bits(·)`-Typen (70.142), `if` (67.769) /
  `let` (82.350) als Kern-Rumpfgerüst (Druck via Lane-09/10-Pipeline).

## 2. Lückentabelle (Konstrukt · Häufigkeit · Beispiel-Span · Owner)

Sortiert nach Häufigkeit. Spans sind `arm-v9.4-a/src`-relativ.

| # | Konstrukt | Häufigkeit | Beispiel-Span | Owner-Lane |
|---|-----------|-----------:|---------------|------------|
| 1 | `throw(Error_…)`-Ausdruck (`Error_Undefined` 19.201, `Unpredictable` 876, `See` 635, …) | 21.014 | `sysregs_autogen.sail:3348` `throw(Error_Undefined())` | Lane 28 |
| 2 | `@` Bitvektor-Konkatenation (`E_vector_append`) | 9.835 | `instrs64_sve.sail:8085` `UInt(i3h @ i3l)` | Lane 22 |
| 3 | `return(…)` (Early-Return-Ausdruck) | 6.663 | `v8_base.sail:18030` `return(addrdesc.fault.statuscode != Fault_None)` | Lane 28 |
| 4 | `undefined` (davon 111 typisiert `(undefined : T)`) | 6.480 | `reset.sail:106` `return(undefined : bits(4))` | Lane 09 |
| 5 | `->` Funktionstypen in `val`-Sigs (`TypKind::Fn` → `conv=None`) | 6.364 | `devices.sail:151` `val __ReadGIC : bits(16) -> bits(32) effect {rmem, rreg}` | Lane 05 |
| 6 | `forall`-Quantoren in Typschemata | 6.079 | `prelude.sail:66` `forall ('a : Type). ('a, 'a) -> bool` | Lane 04 / 05 |
| 7 | `function clause`-Klauseln mit `if`-Guard (`__DecodeA64_*`, `__DecodeT/…`) | 3.996 | `instrs64_sve.sail:105` `function clause __DecodeA64_SVE ((pc, ([bitzero, …] as __opcode)) if SEE < 1037) = {` | Lane 02 / 33 |
| 8 | `as`-Pattern (`[…] as __opcode`) | 3.950 | wie oben, `instrs64_sve.sail:105` | Lane 21 |
| 9 | `match`-Ausdruck | 3.936 | `reset.sail:59` `match __supported_pa_size {` | Lane 21 |
| 10 | `foreach (… by … in inc)` (+ 20 × `in dec`) | 3.254 | `instrs64_sve.sail:53` `foreach (e from 0 to (elements - 1) by 1 in inc) {` | Lane 27 |
| 11 | `assert(…)` | 2.316 | `reset.sail:79` `assert(Have52BitPAExt());` | Lane 28 |
| 12 | `__tup_N`-Tupel-Destrukturierungs-Temps | 1.494 | `sysregs_autogen.sail:127276` `let (__tup_0 : bits(32), __tup_1 : bits(32)) = (…)` | Lane 09 |
| 13 | `register`-Deklarationen (Top-Level 1.392) | 1.419 | `decode_start.sail:41` `register SEE : int`; mit Init `impdefs.sail:867` | Lane 19 |
| 14 | `[r with Feld = …]`-Struct-Updates | 999 | `reset.sail:112` `PMCR_write() = [PMCR_read() with IMP = Zeros(8)];` | Lane 30 |
| 15 | `constraint(…)`-Builtin in Asserts (`'x in {…}`, `'loop_i`-Kids) | 987 | `instrs64_sve.sail:56` `assert(constraint('esize in {16, 32, 64}));` | Lane 25 / 04 |
| 16 | `bitfield`-Typdefs (Top-Level 743) | 744 | `v8_base.sail:41` `bitfield Configuration_Type : bits(32) =` | Lane 30 |
| 17 | `__UNKNOWN_bits` (undefinierte Bits) | 621 | per `grep __UNKNOWN_bits` (621 Stellen, z. B. Decode-Fallbacks) | Lane 22 |
| 18 | `__IMPDEF_*`-Hooks (allein `__IMPDEF_boolean` 4.753 Uses) | ~5.000+ | Dekl. `prelude.sail:334` `val __IMPDEF_boolean : string -> bool`; Use `v8_base.sail:35189` | Lane 25 |
| 19 | `pure {… lean: "…"}`-Backend-Pins (extern-Abbildung, davon `lean:` 16) | 19 `pure` + 7 `monadic` | `prelude.sail:66` `val eq_any = pure {… lean: "BEq.beq" …} : …` | Lane 25 |
| 20 | `effect {…}`-Annotationen an `val` | 22 | `isla_main.sail:48` `… effect {configuration, escape, rmem, rreg, undef, wmem, wreg}` | Lane 08 |
| 21 | `real`-Typ + FP-Helfer (`min_real`, `RoundTowardsZero`, `__UNKNOWN_real`, lem-patches) | ≈160 Stellen | `prelude.sail:81` `val min_real : (real, real) -> real` | Lane 24 |
| 22 | `div` (163) / `ediv` / `mod` in Größen/Arithmetik (`NExp::App` → `conv=None`) | 166 | via `lean:`-Pins teils gemappt (`prelude.sail:102` `lean: "Int.ediv"`) | Lane 23 |
| 23 | String-Literale (6.177) als Dispatch-Schlüssel (`__SetConfig`-Klauseln) | 6.177 Lit. / ~30 Klauseln | `map_clauses.sail:50` `and __SetConfig ((arg, value_name) if arg == "CPTR_EL3.EZ") = {` | Lane 20 |
| 24 | `println` / `print_endline` (Fehlerpfade) | 81 | `map_clauses.sail:46` `println((arg ++ msg));` | Lane 25 |
| 25 | `while … do` / `repeat … until` | 74 / 27 | `instrs64_sve.sail:31051` `while next < elements & not_bool(…) do {` | Lane 27 |
| 26 | `ref`-Parameter (By-Reference) | 62 | `v8_base.sail:2258` `ref R30,` | Lane 09 |
| 27 | `enum` / `struct` / `union`-Top-Levels | 77 / 32 / 3 | `enum`-Beleg per `grep ^enum` (77); `union` 3 | Lane 30 |
| 28 | `overload` (+ 9 `operator`, 5 `infix[r|l]`) | 41 / 14 | `prelude.sail:59` `overload operator --> = {implies}`; `prelude.sail:58` `infixr 1 -->` | Lane 03 |
| 29 | `termination_measure`-Deklarationen | 27 | `termination_measures.sail:41` `termination_measure Reduce__1(op,input,esize,altfp) = 'N` | Lane 26 |
| 30 | `scattered`-Deklarationen (Decode-Tabellen-Köpfe) | 21 | `decode_start.sail:67` `scattered function __DecodeA64_Reserved` | Lane 02 / 11 |
| 31 | `and`-Klauselketten (Multi-Klausel-Funktionen) | 132 | `map_clauses.sail:50ff.` `and __SetConfig …` | Lane 02 |
| 32 | `try` / `catch` + `exit()` | 7 / 8 / 1 | `fetch.sail:143` `try {`; `fetch.sail:275` `exit()` | Lane 28 |
| 33 | `instantiation … with` (Mem-Interface) | 9 | `interface.sail:109` `instantiation sail_mem_read with` | Lane 29 |
| 34 | `sizeof('n)` in Größenposition | 1 | `prelude.sail:251` `Slice(Align_int(unsigned(x), y), 0, sizeof('n))` | Lane 04 |
| 35 | `{|5, 8, 11|}`-Mengen-Constraints | ≥1 | `lem-patches/FPRecpX.sail:52` `esize : {|5, 8, 11|} = 5;` | Lane 04 |
| 36 | `$include` / `$ifdef` / `$else` / `$endif` / `$define` / `$iftarget` (+ `$ifndef`) | 42 | `prelude.sail:43` `$include <smt.sail>` | Lane 02 / 18 |
| 37 | `default Order dec` | 1 | `prelude.sail:41` `default Order dec` | Lane 27 |

Abgrenzung (im Korpus **kein** Druck, trotzdem außerhalb des
Drucker-Subsets — kein Owner-Handlungsbedarf bis Lane 02 Vollgrammatik
sie bräuchte): `mapping`/`mapdef` (0 — Decode/Execute sind
`function clause`s, kein einziges `Mapdef`), `outcome` (0), `newtype`
(0), `private` (0), `mutual`-Stmt (0, nur Kommentar), `var` (0 —
ARM-Code mutiert `let`-Bindungen im Block), `cast` (0), `config`-Stmt
(0, nur Kommentare), `internal_plet/return/assume` (0),
`early_return`-Stmt (0), `impure`-Vals (0), `when`-Guards (0 außerhalb
von String-Literalen), echte `for`-Schleifen (0 außerhalb Strings),
`2^`-NExp (1, abgedeckt via `Pow2`).

## 3. Scanner-Skizze (ungeprüfter Entwurf, E3 — nicht gebaut)

Nutzen: `sail_lex::lex` (Tokenstrom + `TokenKind::name()` als stabile
Zählschlüssel) und `sail_parse::{parse_files_parallel, Def}` (Def-Ebene
mit Spans + `body_src`-Rohtext für die Rumpf-Heuristiken, bis Lane 02
echte `Exp`/`Typ`-Knoten liefert). Zählung zweistufig: exakt auf
Def-/Token-Ebene, heuristisch (Zeilen-/Body-Regex) innerhalb `body_src`.

```rust
#![forbid(unsafe_code)]
//! arm-gap-scanner (Lane-75-Skizze): zaehlt Lean-Subset-Luecken im
//! arm-v9.4-Korpus. Stufe 1 (exakt): sail-lex-Token + sail-parse-Defs.
//! Stufe 2 (heuristisch): Body-Regex in `body_src`, bis Lane 02 echte
//! Exp/Typ-Knoten liefert. Ungepruefter Entwurf (E3, nicht gebaut).

use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct GapCounts {
    /// Konstrukt-Name -> Vorkommen (z. B. "throw", "foreach", "with-update").
    pub hits: BTreeMap<&'static str, u64>,
    /// Konstrukt-Name -> erste Belegstelle als "datei:zeile".
    pub first_span: BTreeMap<&'static str, String>,
}

impl GapCounts {
    pub fn bump(&mut self, key: &'static str, span: &sail_ast::Span, file: &str) {
        *self.hits.entry(key).or_insert(0) += 1;
        self.first_span.entry(key).or_insert_with(|| {
            format!("{file}:{}:{}", span.start_line, span.start_col)
        });
    }
}

/// Stufe 1: Def-Ebene aus `sail_parse::parse_files_parallel`.
/// Jede `Def`-Variante ausserhalb des Drucker-Subsets wird gezaehlt;
/// `body_src` geht an Stufe 2 weiter. `conv::typ_from_ast` entscheidet
/// ueber Typ-Luecken (`None` == Luecke mit Owner Lane 04/05/24).
pub fn scan_defs(files: &[sail_parse::ParsedFile], out: &mut GapCounts) {
    for f in files {
        for d in &f.defs {
            let span = sail_ast::Span::DUMMY; // Platzhalter: echte Spans liefert `SpannedDef` nach Lane-02-Anbindung
            let _ = (d, span, &f.path, out);
            // Skizze der Dispatch-Tafel (vollstaendig erst mit Lane-02-Exp/Typ-Knoten):
            // Def::Register{..}            -> out.bump("register", …)            // Owner Lane 19
            // Def::Type{..} (bitfield…)    -> "bitfield/typedef"                 // Owner Lane 30
            // Def::Scattered{..}           -> "scattered"                        // Owner Lane 02/11
            // Def::Overload{..}            -> "overload"                         // Owner Lane 03
            // Def::Measure{..}             -> "termination_measure"              // Owner Lane 26
            // Def::Instantiation{..}       -> "instantiation"                   // Owner Lane 29
            // Def::Constraint{..}          -> "constraint"                       // Owner Lane 04
            // Def::{Fundef, Mapdef, Impl}  -> body_src an Stufe 2               // Owner je Rumpf-Lane
        }
    }
}

/// Stufe 2 (heuristisch bis Lane 02): Rumpf-Muster in `body_src`.
/// Reihenfolge = Tabelle §2; Wortgrenzen wie in den E3-Zaehlungen.
pub const BODY_PATTERNS: &[(&str, &str)] = &[
    ("throw", "Lane 28"),
    ("return", "Lane 28"),
    ("undefined", "Lane 09"),
    ("foreach", "Lane 27"),
    ("assert", "Lane 28"),
    ("while", "Lane 27"),
    ("match", "Lane 21"),
    ("with-update", "Lane 30"), // `[r with F = …]`
    ("constraint-builtin", "Lane 25/04"), // `constraint(…)`
    ("try/catch/exit", "Lane 28"),
    ("ref-param", "Lane 09"),
];

/// Stufe 1b: Token-Ebene aus `sail_lex::lex` — `kind_name()` pro Token
/// zaehlen; `Throw/Try/Catch/Foreach/While/Assert/Bitfield/Register/…`
/// sind exakte Treffer ohne Grammatik. `Pragma`-Token -> `$`-Direktiven
/// (Owner Lane 02/18), `Real`-Token -> Lane 24.
pub fn scan_tokens(_src: &str, _out: &mut GapCounts) {
    // let res = sail_lex::lex(src);
    // for t in &res.tokens { out.token_bump(t.kind.name(), &t.span); }
}
```

Nach Lane 02 ersetzt Stufe 2 echten Besuchern auf `sail_ast::ExpKind`
(`Throw Try Assert Loop For Var Ref Constraint …`), `PatKind`
(`As …`), `TypKind (`Fn Bidir Exist`), `NExpKind` (`If App`) und
`DefKind` (`Register Measure Instantiation …`); die Owner-Spalte aus §2
wird zur Dispatch-Tafel.

## 4. Top-Lücken (Priorität für Owner-Lanes)

1. **Lane 28 (`throw` 21k / `return` 6,7k / `assert` 2,3k):** ohne
   `SailME`-Flächendeckung übersetzt kein einziges der drei
   50k-Dateien-Shards (`instrs*`, `v8_base`, `sysregs_autogen`).
2. **Lane 22 (`@` 9,8k + 65k `bitzero/bitone`-Decode-Patterns):**
   Bitvektor-Append ist die häufigste nicht abgedeckte Operation im
   Rechenkern (SVE/SME-Schleifenrümpfe).
3. **Lane 27 (`foreach` 3,3k):** der SVE/SME-Standard-Schleifenidiom;
   `inc`-Default + 20 `dec`-Fälle + `default Order dec`
   (`prelude.sail:41`) in einem Aufwasch.
4. **Lane 21 (`as` 4k / `match` 3,9k / `clause`-Guards 4k):** ohne
   Pattern-+Guard-Abbildung bleiben alle `__DecodeA64_*`-Dispatch-Tafeln
   (Lane 33) unübersetzbar.
5. **Lane 05/04 (`->` 6,4k / `forall` 6,1k):** jede zweite `val`-Sig
   fällt heute in `conv=None` (Fn-Typen, Quantoren) — billigster
   relativer Gewinn pro Knotenart, da Sigs klein sind.
6. **Lane 09 (`undefined` 6,5k / `__tup_` 1,5k / `ref` 62):** braucht
   die explizite `undefined`-Regel (kein stilles `sorry`, siehe
   Lane-10-Vertrag) vor allen anderen Rumpf-Lanes.
7. **Lane 25 (`__IMPDEF_*` ~5k Uses + `constraint(…)` 987):** IMPDEF-Hooks
   und Constraint-Asserts sind die Brücke zwischen reinem Modell und
   konfigurierbarem ARM-Verhalten (`impdefs.sail`, `map_clauses.sail`).

## 5. Gültigkeitsnotizen

- Zählungen sind Wort-/Regex-Treffer auf Korpus-Text (Kommentare und
  String-Literale enthalten `when`/`for`/`mapping`-Treffer, die in §2
  herausgerechnet sind; Rest-Rauschen < 1 % bei den Top-10).
- `mapping`-Lücke entfällt faktisch: arm-v9.4 nutzt **keine**
  `mapping`-Definitionen (einziges `mapping`-Vorkommen steht in einem
  String in `event_clauses.sail:92`); String-Dispatch
  (`__SetConfig`, `map_clauses.sail:50`) bleibt Lane-20-Fall.
- `real` betrifft fast nur `prelude.sail`-Vals + `lem-patches/`
  (FP-Rundung); der Integer-/Bitvektor-Kern ist `real`-frei.
- Größte Einzeldatei `sysregs_autogen.sail` dominiert `throw`/`match`
  (generierte Systemregister-Zugriffe) — Shard-Strategie folgt Lane 33.
