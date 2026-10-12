# Lane-67: NoSorry-Bereitschaft — Triage (nur Diagnose, kein Fix)

Stand: 2026-10-13. Basis: Haupt-Worktree auf `a972e3a` (plus uncommittete
Mutable-/Grammatikarbeit, keine Crate-Änderung durch diese Lane).
Lean: 4.35.0 (`/home/simon/.elan/bin/lean`). Alle Läufe via
`/tmp/opencode/sail-gate/gated-run.sh` (MemoryMax gesetzt, Swap 0);
kein Compiler-Umbau, kein ARM-Vollauf.

## 1. Methode

Kleine emittierte Fixtures (simulierte Emitter-Ausgaben für echte ARM-Defs)
einzeln mit `lean -o` kompilieren, dann einzeln durch
`scripts/CheckLeanNoSorry.lean` jagen — Muster nach
`scripts/test-lean-no-sorry.sh` (Referenz). Treiber:
`/tmp/opencode/sail-gate/lane67/lane67-run.sh` (nicht im Repo, Log-Kopie:
`gate-battery.log` in dieser Lane). Fixture-Quellen inline in §2
(reproduzierbar ohne Zusatzdateien).

Gate-Referenzlauf: `gated-run.sh 2G bash scripts/test-lean-no-sorry.sh`
→ **rc=0** (`Good` passiert, `Bad`/`Transitive` werden mit
`Lean proof hole: bad` abgewiesen). Das Gate ist intakt; unten steht,
was es zusätzlich abdeckt — und was nicht.

## 2. Batterie-Ergebnisse (empirisch, `gate-battery.log`)

| Fixture (simuliertes Emitter-Output) | Quelle | Gate | Befund |
|---|---|---|---|
| `GoodEmit`: `def good_emit : Nat := 42` | Kontrolle | rc=0 PASS | kein False-Positive |
| `BadSorryTerm`: `def bad_sorry_term : Nat := sorry` | Literal-Hole im Wert | rc=1 `Lean proof hole: bad_sorry_term` | Scan würde ebenfalls refusen (`emit.rs:320-327`) |
| `BadSorryAx`: `def bad_sorry_ax : Nat := sorryAx Nat true` | **kein** `sorry`/`admit`-Token im Body (`sorryAx : (α : Sort u_1) → Bool → α`, per Probe verifiziert) | rc=1 `Lean proof hole: bad_sorry_ax` | **Lücke G1**: Token-Scan passiert, nur Gate fängt |
| `HelperMacro`: `syntax "myhole" : term` + `macro_rules \| \`(myhole) => \`(sorry)` | Makro-Definition mit quotiertem `sorry` | rc=0 PASS | korrekt: quotiertes `sorry` ist kein elaboriertes Hole |
| `UseMacroHole`: `import HelperMacro` + `def bad_macro_use : Nat := myhole` | Use-Site **ohne** Hole-Token | rc=1 `Lean proof hole: bad_macro_use` | Taktik-/Makro-Holes nur via elaboriertem Gate fangbar |
| `BadOpaqueArm`: `opaque bad_opaque_arm : False := by sorry` | ARM-Muster: opake Dekl. mit Taktik-Hole | rc=1 `Lean proof hole: bad_opaque_arm` | `allowOpaque := true` greift (`CheckLeanNoSorry.lean:15`) |
| `BadTypeHole`: `def bad_ty_hole : (sorry → True) := fun _ => trivial` | Hole in **Typ**-Position, Wert sorry-frei | rc=1 `Lean proof hole: bad_ty_hole` | `info.type.hasSorry` greift; Nebenbefund: naives `def x : sorry := True.intro` elaboriert gar nicht erst (Type mismatch) |
| `AxiomDecl`: `axiom my_unsound_ax : False` | unsound Axiom ohne sorry-Marker | rc=0 PASS | **Lücke G2**: Gate ist axiom-blind |
| `GoodClassical`: `noncomputable def good_classical : Nat := Classical.choose (⟨0, trivial⟩ : ∃ _ : Nat, True)` | sorry-freie Klassik | rc=0 PASS | kein False-Positive |
| `TransMarker`: `import BadSorryTerm` + `def marker_ok67 : Nat := 1` | transitives Hole | rc=1 `Lean proof hole: bad_sorry_term` | `importModules`-Umgebung enthält transitive Konstanten |

Lauf-Kennzahlen: Batterie-Einheit Peak **2,1G**, Swap 0, rc=0 (alle
Erwartungen erfüllt); Compiler-Warnungen `declaration uses 'sorry'`
erscheinen wie erwartet bei allen Hole-Fixtures.

## 3. Triage-Tabelle: Konstrukt → Status → zuständige Lane

Lane-Begriffe (zwei Systeme, hier qualifiziert): **Modul-Lanes** aus der
Code-Doku — Lane 10 = Printer-Kern (`lib.rs`+`conv.rs`+`pats.rs`,
vgl. `emit.rs:21-26`), Lane 14 = Body-Drucker (`exps.rs`), Lane 33 =
`emit.rs`; **Typcheck-Themen-Lanes** aus Lane-31-REPORT — 25
(`E_field`/`E_struct`), 26 (`E_match`), 27 (Vektor-Bounds im App-Arm),
28 (Termination), 29 (Mappings), 30 (Vektor-LExp), Outcome-`with`-Scope
unvergeben.

| # | Lean-Konstrukt aus echten ARM-Defs | Status | Zuständige Lane |
|---|---|---|---|
| T1 | `sorry`-Term im Wert (`BadSorryTerm`) | bereit: Scan refust + Gate flaggt | Lane 33 (Scan) / Gate-Skript |
| T2 | `admit` im Wert | bereit: Scan refust + Gate flaggt | Lane 33 / Gate-Skript |
| T3 | `sorryAx`-Term ohne Hole-Token (`BadSorryAx`) | **Lücke G1**: Scan passiert, nur Gate fängt | Follow-up **Lane 33** (`body_has_hole` um `sorryAx`/`syntheticSorry` erweitern + Unit-Test; hier nur Diagnose) |
| T4 | Makro-/Taktik-expandiertes Hole (`UseMacroHole`) | bereit nur dank Gate (by design) | Gate-Skript bleibt autoritativ (`emit.rs:84-86`); kein Fix nötig |
| T5 | `opaque … := by sorry` (ARM-opake Dekls) | bereit: Gate flaggt via `allowOpaque` | Gate-Skript; Produzentenseite Lane 33 (nie opak emittieren) |
| T6 | Hole in Typ-Position (`BadTypeHole`) | bereit: `type.hasSorry` flaggt | Gate-Skript |
| T7 | Transitives Hole (`TransMarker`) | bereit: `importModules`-Umgebung | Gate-Skript |
| T8 | `axiom … : False` (`AxiomDecl`) | **Lücke G2**: Gate passiert | Follow-up Gate-Härtung (z. B. `collectAxioms`/`#print axioms`-Äquivalent oder `axiom`-Quellscan; Lane zu vergeben). Heute **kein Producer**: `sail-lake` emittiert nie `sorry` (Test `generator_never_emits_sorry`, `NO_SORRY_NOTE`), Bodies sind Terme (keine Commands) |
| T9 | `Classical.choose` u.ä. (sorry-frei) | bereit: kein False-Positive | — |
| T10 | Quantifizierte Schemes (Outcome `with 'pa : Type`, polymorphe ARM-Funktionen; `read_write_v1.sail:142-186`) | bereit (Refusal, kein Hole): `Unsupported` statt stiller Lücke (`emit.rs:170-179`, `281-289`) | Outcome-`with`-Scope-Follow-up (unvergeben, Lane-31-Empfehlung) + Binder-Lowering Lane 33 |
| T11 | Nicht-translatierbare Body-Shape (`body: None`) | bereit (Refusal): `emit.rs:348-355`, `lib.rs:682-684` | Lane 33 / Lane 10 |
| T12 | `undefined_*` / `E_undef` (ARM-`unpredictable`) | bereit (Refusal): `exps.rs:701` | Lane 14 + Semantik-Entscheid Follow-up (**kein** `sorry`-Default!) |
| T13 | `Loop`/`Block`/`VectorAppend`/`Sizeof`/`Constraint`/`Config`/`Var`/`InternalValue`/`InternalAssume` | bereit (Refusal): `exps.rs:694-708` | Lane 14, flankierend 25/27 |
| T14 | Loop-Kombinatoren ohne Desugaring (`doc_loop`) | bereit (Refusal): `exps.rs:815-821` | Lane 14/27 |
| T15 | Operatoren ohne `lean(...)`-Spelling | bereit (Refusal): `exps.rs:838-850` | Extern-Tabelle (Alt-Lane unvergeben) |
| T16 | Real-Literale (ARM-Float-Helfer) | bereit (Refusal): `conv.rs:171` → `exps.rs:532-533` | Real-Support-Follow-up (`sail-real`) |
| T17 | `Exist`/`Bidir`/`Unknown`-Typen, NExp `If`/`Id`/`App`, `TypArg Bool` | bereit (Refusal via `None`): `conv.rs:56`, `130-143` | Lane 10 (+ Typ-/Solver-Themen) |
| T18 | `P_not`/`P_or`/`P_vector_subrange`/`P_string_append`, breitenlose `P_id` | bereit (Refusal): `pats.rs:281-296`, `335-345` | Lane 10 |
| T19 | `E_match` (ARM `match request.access_kind`, `read_write_v1.sail:111-115`) | bereit (Refusal, heute Typcheck-Catch-all; Lane-31 B3) | Lane 26 (alt) |
| T20 | Vektor-LHS `res[i] = …` (`vector.sail:478`) | bereit (Refusal, heute Typcheck B2) | Lane 30 (alt); emissionsseitig später T11-Pfad |
| T21 | Register (ARM-hunderte) | bereit by construction: `register_entry`, nie `def` (`emit.rs:268-307`); Templates sorry-frei | Lane 33 + `sail-lake` |
| T22 | Typedefs (Enums/Structs/Bitfields) | bereit: `Abstract`→`Unsupported` (E2), Output sorry-frei (Test `rendered_typedefs_never_contain_sorry`) | `sail-typedef` (kein Lane-Eintrag in `rig-triage.py` → zu vergeben) |
| T23 | `partial`/`noncomputable`-Präfixe (ARM-Rekursion) | bereit, terminierungsblind aber kein Proof-Hole: Gate passiert korrekt (`lib.rs` opts, `tests.rs:313-323`) | Lane 10 + Termination (28 alt) |
| T24 | Sail-Namen `sorry`/`admit` | bereit: Sanitizing zu `sorry'`/`admit'` (`lib.rs:797-815`), kein akzidentelles Hole-Token | Lane 10 |

## 4. Urteil

NoSorry-Bereitschaft ist **bedingt gegeben**: Jedes heute emittierbare
Hole wird vom elaborierten Gate gefangen (T1/T2/T4–T7, Transitivität),
jede heute nicht-emittierbare ARM-Shape wird laut mit
`Error::Unsupported` verweigert (T10–T22, T24). Zwei benannte Lücken,
beide ohne heutigen ARM-Pfad, beide mit Follow-up-Owner:
**G1** (`sorryAx` passiert `body_has_hole` → Lane 33) und **G2**
(Axiom-Blindheit des Gates → Gate-Follow-up). Kein Fix in dieser Lane.

## 5. Dateien dieser Lane (nur `evidence/2026-10-13/lane-67/`)

- `REPORT.md` (diese Datei)
- `gate-battery.log` — Kopie des Batterie-Logs (Gate-Zeilen §2, Peak 2,1G)
