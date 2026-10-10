# Lane 47 — AST-Unify: Migrationstabelle (Analyse, kein Code-Umbau)

Status: `lane/47-ast-unify`, QUEUED. **E3 gilt**: keine Builds/Tests
(`cargo`, `lake`, `sail` verboten — OOM-Schutz, DECISIONS.md E3).
Diese Datei ist reine Analyse. Fremde Crates werden NICHT umgeschrieben.
Start der Umsetzung erst nach Integrator-Merge aller Phase-1-Lanes
(Spec `lanes/47-ast-unify.md`).

`sail-ast` (Lane 01, `crates/sail-ast/src/`) ist die einzige Wahrheit:
`ast.rs` (Node<K>, Name, Kind, Lit/LitKind, NExp/NExpKind, TypArg/TypArgKind,
Typ/TypKind, Constraint/ConstraintKind, QuantItem, TypSchm, Pat/PatKind,
TypPat, Exp/ExpKind, LExp, FExp, PExp, Funcl, Mapcl, RecOpt, TAnnotOpt,
TypeUnion, IndexRange, Extern, ValSpec, TypeDef/TypeDefKind, DecSpec, Fixity,
Subst, ScatteredDef, Def/DefKind, FunclSet, Ast),
`span.rs` (FileId, Span mit DUMMY/merge),
`id.rs` (NodeId), `interner.rs` (Symbol/SimpleInterner).

## 1. Swap-Verträge (`*_NEEDED.md`, 5 Dateien)

| Vertrag | Stand-in heute | Ziel in `sail-ast` | Kern des Vertrags |
|---|---|---|---|
| `crates/sail-lex/LEXER_NEEDED.md` | `sail-lex/src/span.rs`: `FileId(pub u32)`, `Span{file,start,end}` + DUMMY/merge | `sail_ast::{FileId,Span}` Feld-für-Feld | Option (a) `span.rs` löschen + Re-Export, oder (b) lokale Typen behalten + Konvertierung an der Grenze. Kein File-Table hier. |
| `crates/sail-parse/AST_NEEDED.md` | `sail-parse/src/mini_ast.rs`: `Def` (17 Varianten, Bodies als Rohtext), `ParsedFile`, `SpannedDef`; `src/span.rs`: `Span{file:String,start_byte,end_byte,start_line,start_col,end_line,end_col}`, `Spanned<T>`, `ParseError` | `Def/DefKind` (DEF_type/fundef/mapdef/impl/let/constraint/overload/fixity/default/scattered/register/measure/outcome/instantiation/pragma + DEF_private/DEF_attribute-Wrapper), `Ast`, `sail_ast::Span`, Fehlertyp aus Lane error-diagnostics | `parse_file` gibt künftig `sail_ast::Ast` zurück; Header-Parsing bleibt, Bodies werden echte `Exp`/`Typ`; `file_start`/`file_end`-Wrapping bleibt (als `DEF_pragma` oder Metadaten, mit Lane 11 abzustimmen); `mini_ast.rs` löschen, `span.rs` durch `sail_ast::Span` ersetzen. |
| `crates/sail-resolve/RESOLVE_NEEDED.md` | `sail-resolve/src/mini_ast.rs`: `Ast{defs}`, `Def::{Type,ValSpec,Register,Mapping,Other}`, `TypeDef/TypeDefKind::{Struct,Enum(Vec<CtorDecl>),Union(Vec<CtorDecl>),Abbrev}`, `CtorDecl`, `ValSpec`, `RegisterDef`, `MappingDef`, `Span{line,col}` | `Ast`/Definitionsliste, `TD_struct`/`TD_enum`+`E_ctor`/`TD_variant`+`Tu_*`/`TD_abbrev`-Familie, `VS_val_spec`, `DEC_reg`, `DEC_mapping`/`MAP_*`, `DEF_fundef`/`DEF_scattered`/`DEF_pragma`, `l`/Loc | Semantikvertrag: getrennte Namespaces, order-independent (bind→check), Ctor-vs-Type-Kollision = Warnung + `compute_ctor_renames`-Eintrag (Signatur frozen: rein, deterministisch, `_ctor`/`_ctor{N}`), Duplikat-Ctor über Owner = `DuplicateCtor` + Rename-Eintrag. Swap: `sail-ast`-Dep eintragen, `mini_ast::*`→`sail_ast::*`, `mini_ast.rs` löschen. |
| `crates/sail-ty/TY_NEEDED.md` | `sail-ty/src/nexp.rs`: `Nexp` (11), `NConstraint` (10), `MiniTyp` (8), `MiniExp` (7), `TypQuant`; `src/kind.rs`: `Kind::{KInt,KNat,KBool,KType}`, `KindEnv`; `src/env.rs`: `TypeEnv`; `src/error.rs`: `Span{file:u32,start,end}` + DUMMY | `Nexp_aux`/`NExpKind`, `NC_aux`/`ConstraintKind`, `Typ_aux`/`TypKind`, `Exp_aux`/`ExpKind` (Subset), `TypQ_aux`+`kind_aux::{K_int,K_bool,K_type}`, `Env.t`, `Span` | `KNat` ist lokale Refinement-Unterart für Lean-`Nat`/`Int`-Mapping (Lane 04/10): beim Swap als Wrapper behalten oder nach `sail-nexp` verschieben, niemals still auf `K_int` kollabieren. `depth`/`free_vars`/`as_const`/`as_bool`/Fuel-Metrik überleben als dünne Wrapper. `Solver`-Trait + `CachingSolver` unangetastet (Lane 06 implementiert). |
| `crates/sail-regs/REGS_NEEDED.md` | `sail-regs/src/lib.rs`: `MinimalDef::{Register{name,typ:SailTyp,init:Option<String>},Other}`, `SailTyp(String)` | `DefKind::Register(DecSpec)` (`crates/sail-ast/src/ast.rs`): `name=DecSpec.name`-Symboltext, `typ=DecSpec.typ` gerendert, `init=DecSpec.init:Option<Exp>` gerendert (`None` = kein `= …`) | Konversion fest vorgegeben (siehe REGS_NEEDED.md §Mapping). Synonym-Expansion + Bitfield-Basis bleiben Typechecker-Job; Caller übergibt expandierte Schreibweise. `TypKind`-Pretty-Printing kommt aus Lane 10. |

## 2. Migrationstabelle Minimaltyp → `sail-ast`-Ziel je Lane

Zählregel: eine Zeile = ein `pub struct/enum/type`, das beim Swap durch
`sail-ast` ersetzt/gelöscht wird. Reine Backend-/Pipeline-IR
(`RegisterInfo`, `Shard`-Pipeline, `Solver`, Fuel-Limits, Printer-`Options`)
steht in §3 und zählt NICHT mit.

### Lane 16 — `sail-lex` (2)

| # | Minimaltyp heute | `sail-ast`-Ziel | Bemerkung |
|---|---|---|---|
| 1 | `sail-lex/src/span.rs:FileId(pub u32)` | `sail_ast::FileId` | feldidentisch, inkl. Display nur lokal |
| 2 | `sail-lex/src/span.rs:Span{file,start,end}` + DUMMY/merge | `sail_ast::Span` | feldidentisch; Swap-Option (a)/(b) s. LEXER_NEEDED §1 |

### Lane 02 — `sail-parse` (5)

| # | Minimaltyp heute | `sail-ast`-Ziel | Bemerkung |
|---|---|---|---|
| 3 | `sail-parse/src/span.rs:Span{file:String,start_byte,end_byte,start_line,start_col,end_line,end_col}` | `sail_ast::Span` (+ File-Table aus Driver-Lane) | heute Pfad-String statt `FileId`; beim Swap `file`→`FileId` + Zeile/Spalte aus File-Table; Feldnamen bewusst angeglichen |
| 4 | `sail-parse/src/mini_ast.rs:Def` (Val/Fundef/Mapdef/Impl/Type/Let/Constraint/Overload/Fixity/Default/Scattered/Register/Measure/Outcome/Instantiation/Pragma/Other) | `sail_ast::DefKind::{Val,Fundef,Mapdef,Impl,Type,Let,Constraint,Overload,Fixity,Default,Scattered,Register,Measure,LoopMeasures,Outcome,Instantiation,Pragma,…}` | Bodies heute Rohtext (`*_src`); nach Swap echte `Exp`/`Typ`-Knoten. `Other` entfällt (vollständige Grammatik). `private`→`DEF_private`, `$[…]`→`DEF_attribute`, Klausel-Gruppierung `and`→ein `DEF_fundef` |
| 5 | `sail-parse/src/mini_ast.rs:ParsedFile{path,defs}` | `sail_ast::Ast{defs}` + File-Wrapper (`file_start`/`file_end` als `DEF_pragma`, Abstimmung Lane 11) | Shard-Marker bleiben erhalten |
| 6 | `sail-parse/src/mini_ast.rs:SpannedDef{def,span}` | `sail_ast::Node<DefKind>` (`id`+`span`+`kind`) | generationaler `NodeId` kommt hinzu |
| 7 | `sail-parse/src/mini_ast.rs:PragmaArg::{Line,Structured}` | `sail_ast::DefKind::Pragma{key,payload}` | strukturiertes `{k=v}`-Parsing spätere Verfeinerung |

### Lane 03 — `sail-resolve` (9)

| # | Minimaltyp heute | `sail-ast`-Ziel | Bemerkung |
|---|---|---|---|
| 8 | `sail-resolve/src/mini_ast.rs:Span{line,col}` | `sail_ast::Span` | heute nur Zeile/Spalte; Byte-Range kommt beim Swap |
| 9 | `sail-resolve/src/mini_ast.rs:Ast{defs}` | `sail_ast::Ast{defs}` | flache order-independente Liste bleibt |
| 10 | `sail-resolve/src/mini_ast.rs:Def::{Type,ValSpec,Register,Mapping,Other}` | `DefKind::{Type,Val,Register,Mapdef,…}` (Rest → `Fundef`/`Scattered`/`Pragma`/…) | `Other` muss mind. Namen liefern |
| 11 | `TypeDef{name,kind,span}` | `sail_ast::TypeDef` (`TypeDefKind`) | Name + Kind |
| 12 | `TypeDefKind::{Struct,Enum,Union,Abbrev}` | `TypeDefKind::{Record,Enum,Variant,Abbrev,Abstract,Bitfield}` | `Struct`→`Record`; `Abbrev` deckt `Abbrev/Alias/Bitfield`-Namen ab (Details → Lane 30) |
| 13 | `CtorDecl{name,span}` | `TypeUnion{typ,ctor}` / `Enum`-Member-`Name` | Payload-Typen out of scope Lane 03 |
| 14 | `ValSpec{name,span}` | `sail_ast::ValSpec` (`VS_val_spec`) | Extern-Info später |
| 15 | `RegisterDef{name,span}` | `DecSpec` via `DefKind::Register` | Semantik → Lane 19 |
| 16 | `MappingDef{name,span}` | `DefKind::Mapdef` / `Mapcl` | Encode/Decode → Lane 20 |

### Lane 05 (+04) — `sail-ty` / `sail-nexp`-Rand (9)

| # | Minimaltyp heute | `sail-ast`-Ziel | Bemerkung |
|---|---|---|---|
| 17 | `sail-ty/src/nexp.rs:Nexp::{Const,Var,Add,Sub,Mul,Neg,Exp,Div,Mod,Abs,If}` | `sail_ast::NExpKind::{Constant,Var,Sum,Minus,Times,Exp,Neg,If,App,Id}` | `Div/Mod/Abs` = `Nexp_app`-Knoten dort; `If`-Cond wird `Constraint`; ggf. `Nexp_id`/`Nexp_app` dazu |
| 18 | `sail-ty/src/nexp.rs:NConstraint::{True,False,Eq,Neq,Ge,Gt,Le,Lt,And,Or,Not}` | `sail_ast::ConstraintKind::{True,False,Equal,NotEqual,Ge,Gt,Le,Lt,And,Or,App,Id,Var,Set,…}` | `NC_set`/`NC_app`/`NC_var` bei Bedarf ergänzen |
| 19 | `sail-ty/src/nexp.rs:MiniTyp::{Int,Bool,Bit,Bits,Range,Named,Fun,Tuple}` | `sail_ast::TypKind::{Unknown,Id,Var,Fn,Bidir,Tuple,App,Exist}` | `Range`≈`atom`-bounded int; `Named`≈`Typ_app`/`Typ_id`; Existentials → Lane 09 |
| 20 | `sail-ty/src/nexp.rs:TypQuant{quants,constraint,typ}` | `sail_ast::TypSchm{quant,typ}` + `QuantItem::{Id(KOpt),Constraint}` | `quants:Vec<(String,Kind)>` ≈ `quant_item`-Teleskop |
| 21 | `sail-ty/src/nexp.rs:MiniExp::{IntLit,BoolLit,Var,Let,App,Tuple,Annot,If}` | `sail_ast::ExpKind` (Subset: `Lit/Id/Let/App/Tuple/Typ/If`) | Vollport Lane 02/03; Match-Arme wachsen mit |
| 22 | `sail-ty/src/kind.rs:Kind::{KInt,KNat,KBool,KType}` | `sail_ast::Kind::{Type,Int,Bool}` | `KNat` NICHT löschen: Wrapper-Refinement für Lean-`Nat` (Lane 04/10), erase → `K_int` nur an der Konvertierung |
| 23 | `sail-ty/src/kind.rs:KindEnv` | `sail_ast` Kind-Kontext / `Env.t`-Ausschnitt | reine Bindungs-Map, kein AST-Knoten |
| 24 | `sail-ty/src/env.rs:TypeEnv{vals,locals,registers}` | `Env.t` (`env.ml`: val specs, registers, scattered) | Snapshots bleiben `Clone+Send+Sync` für Worker |
| 25 | `sail-ty/src/error.rs:Span{file:u32,start,end}` + `DUMMY=0/0/0` | `sail_ast::Span` (+ Lane-35-Fehlertyp) | DIVERGENT: `file` ist `u32` statt `FileId`, DUMMY-Sentinel abweichend — beim Swap zuerst angleichen |

### Lane 19 — `sail-regs` (2)

| # | Minimaltyp heute | `sail-ast`-Ziel | Bemerkung |
|---|---|---|---|
| 26 | `sail-regs/src/lib.rs:MinimalDef::{Register{name,typ,init},Other}` | `sail_ast::DefKind::Register(DecSpec{typ,name,init})` | 1:1-Konversion steht in REGS_NEEDED.md |
| 27 | `sail-regs/src/lib.rs:SailTyp(String)` | `sail_ast::Typ` (gerendert) + Lane-10-Printer | nur Schreibweise (`bits(64)`), kein AST-Knoten |

### Lane 10 — `sail-lean` Drucker-Subset (4)

| # | Minimaltyp heute | `sail-ast`-Ziel | Bemerkung |
|---|---|---|---|
| 28 | `sail-lean/src/lib.rs:Span{line,col}` | `sail_ast::Span` | nur für `Error::Unsupported`; nach Swap konvertieren, Drucker-Signatur sonst stabil |
| 29 | `sail-lean/src/lib.rs:Nexp::{Const,Var,Sum,Minus,Times,Neg,Pow2}` | `sail_ast::NExpKind` (Subset) | Drucker-Subset; `Pow2`=`Exp`, Div/Mod/Abs fehlen (via `App`) |
| 30 | `sail-lean/src/lib.rs:Typ::{Unit,Int,Nat,Bool,String,Bit,BitVec,Option,List,Result,Tuple,Vector,Var,Named}` | `sail_ast::TypKind` + `TypArgKind` | `Nat`↔`KNat`-Frage (s. #22); `Bit(String)`-Payload nur Druck-Detail |
| 31 | `sail-lean/src/lib.rs:TypArg::{Typ,Nexp}` | `sail_ast::TypArgKind::{Typ,NExp,Bool}` | `Bool`-Arg fehlt im Drucker-Subset — beim Swap ergänzen |

### Lane 21/22/23 — Pattern/Bitvec/Int-Subset (3)

| # | Minimaltyp heute | `sail-ast`-Ziel | Bemerkung |
|---|---|---|---|
| 32 | `sail-patterns/src/lib.rs:Pat::{Wild,Var,Lit,App,Tuple,Vector,VectorConcat,Or,As,Typ,VarTyp,Struct,List,Cons,StringAppend}` | `sail_ast::PatKind::{Lit,Wild,Or,Not,As,Typ,Id,Var,App,Vector,VectorConcat,VectorSubrange,Tuple,List,Cons,StringAppend,Struct}` | `Not`/`VectorSubrange` fehlen lokal — beim Swap ergänzen; `Var(String)` vs `Var{pat,typ_pat}` + `TypPat` trennen |
| 33 | `sail-bitvec/src/pat.rs:Pat::{Wild,Lit,Id,Typ,Vector,VectorConcat}` (je +`span`) | `sail_ast::PatKind`-Subset (s. oben) | Bitvec-Sicht mit `width`-Annotation; nach Swap `Typ`-Ascription + `width`-Ableitung auf `Typ` umstellen |
| 34 | `sail-int/src/nexp.rs:NExp::{Constant,Var,Add,Sub,Mul,Neg,Abs,Pow2,Div,Mod}` | `sail_ast::NExpKind` | kein `If`/`Id`/`App`-allgemein; `Pow2`=`Exp`; `Facts`-Kontext bleibt Lane-04-Logik |

### Lane 20/25/07 — Mapping/Extern/Rewrite-Subset (9)

| # | Minimaltyp heute | `sail-ast`-Ziel | Bemerkung |
|---|---|---|---|
| 35 | `sail-mapping/src/ast.rs:Mpat::{Lit,Var,App,Tuple,StrAppend,VecAppend,Wild}` | `sail_ast::{Pat,MapclKind}`-Seite (`PExp`/`Mapcl`) | `MPat_pat`/`MPat_when`-Trennung kommt beim Swap |
| 36 | `sail-mapping/src/ast.rs:Pat::{Lit,Var,App,Tuple,StrAppend,VecAppend,Wild}` | `sail_ast::PatKind` | `pat_of_mpat`-Ergebnis; danach löschen |
| 37 | `sail-mapping/src/ast.rs:Exp::{Id,Lit,App,Tuple,Let,Match,Exit,Bool,Other}` | `sail_ast::ExpKind` | Minimalsprache für `realize_mappings`; `Other(String)` entfällt |
| 38 | `sail-mapping/src/ast.rs:MappingClause::{Bidir,Forwards,Backwards}` | `sail_ast::MapclKind::{Bidir,Forwards,Backwards}` | Feldnamen bereits angeglichen |
| 39 | `sail-mapping/src/ast.rs:MappingDef{name,typ1,typ2,clauses}` | `sail_ast::DefKind::Mapdef{name,annot,clauses}` + `TypSchm` (`Typ_bidir`) | `typ1/typ2: String` → echte `Typ`-Knoten |
| 40 | `sail-mapping/src/ast.rs:FunDef/ValSpec/Arm` | `sail_ast::{Funcl,ValSpec,PExp}` | realisierte Funktionen; `is_last` (`mapping_last`-Attribut) als Metadatum erhalten |
| 41 | `sail-extern/src/lib.rs:Span{line,col}` | `sail_ast::Span` | wie #28, nur Fehlerposition |
| 42 | `sail-rewrite/src/ast.rs:Def{name,kind,body}` + `DefKind::{Function,Valspec,Register,Type,Other}` | `sail_ast::DefKind` | shard-lokale Hülle; `kind` wird echte Diskriminante |
| 43 | `sail-rewrite/src/ast.rs:Stmt::{Assign,TupleAssign,Undefined,Expr,Return}` | `sail_ast::ExpKind`/`LExpKind` | `String`-Payloads → echte `Exp`/`LExp`; `Undefined`-Rewrite-Eingabe als `ExpKind`-Fall |

### Span-Kopie `sail-bitvec` (2, feldidentisch zu #1/#2)

| # | Minimaltyp heute | `sail-ast`-Ziel | Bemerkung |
|---|---|---|---|
| 44 | `sail-bitvec/src/span.rs:FileId(pub u32)` | `sail_ast::FileId` | Kopie, dependency-frei |
| 45 | `sail-bitvec/src/span.rs:Span{file,start,end}` + DUMMY/merge | `sail_ast::Span` | Kopie inkl. DUMMY/merge |

## 3. Kein Duplikat — bewusst behalten (nicht löschen)

- `sail-nexp` (`expr/lean/pred/simplify`): Lane-04-Algebra (Folding,
  Normalformen, `provably_*`, `Nat`-vs-`Int`-Mapping). Daten wandern nach
  `sail-ast::NExpKind/ConstraintKind`, Logik bleibt hier. Kein AST-Duplikat.
- `sail-ty`: `Solver`-Trait + `CachingSolver`, `CheckLimits`/Fuel,
  `TypedDefSummary`, `TypeErrorKind` (→ Lane 35 verfeinert, nicht 1:1 gelöscht).
- `sail-regs`: `RegisterDef/RegisterInfo/InitialValue`, `lean_of_sail_typ`,
  `regstate`, `initial_values` (Backend-IR aus `pretty_print_lean`/`state.ml`).
- `sail-rewrite`: `Shard{id,defs}` + Chunking (Pipeline-Granularität, Lane 11-nah).
- `sail-mapping`: `realize_mappings`-Logik, `mapping_patterns`-Analyse.
- `sail-patterns`: `Arm{pat,has_guard}`, `ScrutineeType`, `CheckReport`,
  `EnumEnv` (Analyse-Kontext, kein AST-Knoten).
- `sail-lean`: `Effect/FunSig/Options/LeanPrinter`, `Lit`, Monad-Mapping
  (Drucker-eigen; nur `Span/Nexp/Typ/TypArg` aus §2 werden ersetzt).
- `sail-bitvec`: `Lit`, `BitvecError`, Render-/Width-Logik (nur `span`+`pat` wandern).
- `sail-parse`: `Spanned<T>`, `ParseError` → Lane error-diagnostics (35),
  nicht `sail-ast` (in §2 nur als Anmerkung, nicht als Löschzeile gezählt).

## 4. Reihenfolge-Vorschlag nach dem Integrator-Merge (Phase 1 komplett)

Blockiert bis der Integrator alle Phase-1-Lanes gemergt hat. Danach,
in dieser Reihenfolge (jede Stufe eigener Commit, Tests zentral durch
Integrator — E3):

1. **Spans/FileIds** (#1, #2, #8, #25, #28, #41, #3, #44, #45):
   `sail-ast::Span/FileId` überall übernehmen (DIVERGENZEN zuerst:
   `sail-ty::error::Span`, `sail-parse::span::Span`,
   `sail-resolve::Span`). Reine Mechanik, keine Semantik. Danach
   `LEXER_NEEDED §1` abhaken (Option a oder b je Crate festhalten).
2. **Nexp/Constraint + Kind** (#17, #18, #22, #34, #29):
   `sail-ty::nexp` → `sail_ast::NExp/Constraint` + `sail-nexp`-Adapter
   (`depth`/`free_vars`/`as_const`/`as_bool` als Wrapper, `KNat`-Regel
   festschreiben). `sail-int`/`sail-lean`-Subsete nachziehen.
3. **Typ/TypSchm/Env** (#19–#21, #23, #24, #30, #31):
   `MiniTyp/TypQuant/TypeEnv/KindEnv` → `Typ/TypSchm/QuantItem/Env`-Sicht;
   `MiniExp` → `ExpKind`-Subset (Match-Arme wachsen lassen). `Solver`-Trait unberührt.
4. **Pat/Exp/Lit/Mapping-Kerne** (#32, #33, #35–#40, #43):
   `sail-patterns`, `sail-bitvec::pat`, `sail-mapping`, `sail-rewrite::Stmt`
   auf `Pat/Exp/Mapcl/Funcl` umstellen.
5. **Top-level Defs** (#4–#7, #9–#16, #26, #27, #42):
   `sail-parse::mini_ast` → `Ast/DefKind`; `sail-resolve::mini_ast` →
   echte Knoten (Semantikvertrag §1 beachten, `compute_ctor_renames`
   frozen); `sail-regs::MinimalDef` → `DecSpec`; `sail-rewrite::Def`
   zuletzt. `mini_ast.rs`-Dateien löschen, `sail-ast`-Dep eintragen.
6. **Abschluss**: alle 5 `*_NEEDED.md` als erledigt markieren (oder löschen
   falls Lane-Konvention das vorsieht), JSON-Roundtrip (`Ast::to_json`/
   `from_json`) an den Grenzen verdrahten, Done-Kriterium prüfen:
   kein `*_NEEDED.md` mehr offen, ein AST im Workspace.

## Anhang — Duplikat-Zahl

Enge Zählung (Tabellenzeilen #1–#45, §3-Backend-IR ausgeschlossen):
**45 Minimaltyp-Duplikate**.
(`Spanned<T>`/`ParseError`/Analyse-/Printer-IR nicht mitgezählt;
`sail-nexp`-Logik bleibt bestehen, nur ihre Datenseite wandert.)
