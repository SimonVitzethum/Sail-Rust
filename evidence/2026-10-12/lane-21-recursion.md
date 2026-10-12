# Lane-21: Rekursions-Audit-Reste (Audit O-7)

**Stand:** 2026-10-12 · Branch `lane-21` auf `a972e3a` · Worktree `worktrees/lane-21`
**Scope-Regel:** `canonical_body.rs` / `program.rs` wurden NICHT editiert (Lanes 16/19
arbeiten dort logisch). Geschrieben wurden nur: dieser Report,
`crates/sail-ty/src/conv.rs` (kleine iterative Ersetzungen),
`crates/sail-ty/src/recursion.rs` (NEU), `crates/sail-ty/src/lib.rs`
(eine `pub mod`-Zeile als Verdrahtung), Tests.
**Zähl-Hinweis:** Auftrag nannte „83× `.clone()` canonical_body, 44× program.rs".
Gemessen auf `a972e3a` (ripgrep, exakte Treffer): **61× `.clone()`** in
`canonical_body.rs` (1414 Zeilen), **24× `.clone()`** in `program.rs`
(1119 Zeilen). Inkl. `to_vec()`/`.cloned()`/`to_owned()`/`to_string()`:
70 Treffer canonical_body, 47 program. Die Differenz (~22/~20) lässt sich
nicht aus dem Stand erklären (evtl. älterer Stand oder Testdateien
mitgezählt); die Tabellen unten listen **alle gemessenen Stellen**
vollständig, sodass keine Stelle ungeprüft bleibt.

## 1. Methode

1. Alle `.clone()`/`to_vec()`/`.cloned()`/`to_owned()`-Stellen per grep mit
   umgebender `fn` inventarisiert.
2. Native Rekursion geprüft: Funktionskörper nach Selbstaufruf durchsucht
   (`canonical_body.rs`, `program.rs`: **keine einzige selbstrekursive
   Funktion** — die zwei Treffer `get`/`mismatch` bzw. `prove`/`new` sind
   Namensgleichheiten mit Fremdmethoden, verifiziert durch Lektüre).
   Beide Dateien arbeiten mit expliziten Work-Stacks (`pairs`/`work`/`tasks`,
   `while let ... pop()`).
3. `conv.rs`: alle Konverter sind **nativ rekursiv** (Selbstaufruf-Zählung:
   `nexp_to_ast` 15, `nexp_from_ast` 15, `constraint_to_ast` 5,
   `constraint_from_ast` 5, `typ_to_ast` 3, `typ_from_ast` 3,
   `minipat_to_ast/from_ast` je 2, `minilexp_to_ast/from_ast` je 5,
   `miniexp_to_ast/from_ast` je 31).
4. Empirischer 10.000-Tiefen-Beleg auf 128-KiB-Worker-Threads (gleiches
   Budget wie die bestehenden `canonical_program`-Tiefentests): temporäre
   Sonden-Datei, je Sonde eigener Prozess (da `SIGABRT` den Prozess tötet),
   via `/tmp/opencode/sail-gate/gated-run.sh`.

**Urteils-Skala:** `BEGRENZT` = Ausführung in expliziter Work-Stack-Schleife
oder über deklarations-/quellengrößenbeschränkte Daten (Tiefe wächst nicht
mit dem Call-Stack). `UNBEGRENZT` = native Rekursion über Eingabetiefe
(Stack wächst linear mit der Eingabe → `SIGABRT`-Risiko).

## 2. 10.000-Tiefen-Beleg (vor jeder semifinalen Behauptung)

Temporäre Sonden (`lane21_probe.rs`, nach Beleg wieder entfernt, Logik als
sichere Tests in `tests/lane21_recursion.rs` überführt), je einzeln via Gate:

| Sonde (10_000 tief, 128 KiB Stack) | Ergebnis |
|---|---|
| `Nexp::Add`-Kette → `Nexp::depth()` | **stack overflow, SIGABRT** (Prozess-Abort) |
| gleiche Kette → `conv::nexp_to_ast` | **stack overflow, SIGABRT** |
| `NConstraint::And`-Kette → `NConstraint::depth()` | **stack overflow, SIGABRT** |
| gleiche Kette → `conv::constraint_to_ast` | **stack overflow, SIGABRT** |

Zusatzbefund beim Härten der finalen Tests: das **Droppen** einer 10_000
tiefen `Box`-Kette (`Nexp`/`NConstraint`, abgeleitetes `Drop`) overflowt den
128-KiB-Stack ebenfalls (rekursives Drop). Die finalen Tests `mem::forget`ten
die Tiefenwerte daher bewusst; `sail-ast`-Knoten haben dafür bereits
`storage::drop_definition` (iterativ) — ein Spez-Punkt (§6, Prio 2).

**Gegenbeleg nach Fix:** `tests/lane21_recursion.rs`
(`iterative_guards_reject_10k_inputs_on_a_small_stack`,
`..._accept_shallow_inputs_on_a_small_stack`) + Unit-Tests in
`recursion.rs` — alle 10k-Eingaben werden auf 128 KiB **zurückgewiesen
(statt abzustürzen)**. Volle `sail-ty`-Suite grün via Gate, siehe §5.

## 3. `canonical_body.rs` — alle 61 `.clone()`-Stellen (READ-ONLY auditiert)

Keine native Rekursion in der Datei; alle Stellen laufen in Work-Stack-
Schleifen oder einmalig pro Deklaration. Clones sind überwiegend
`Arc`-Inkremente (`Type`/`Scheme`/`Scope` bestehen aus `Arc`s) oder kleine
`Copy`-nahe Strukturen (`Scalar`, `Name`). **Urteil global: BEGRENZT** —
Einzelbegründung je Gruppe:

| Zeilen | umgebende fn | Was wird geklont | Tiefe | Urteil |
|---|---|---|---|---|
| 46, 64 | `Scalar::resolved` | `self: Scalar` (Type=Arcs+Vec, billig), `bound: Scalar` | `loop` über Subst.-Kette, Kettenlänge ≤ Binderzahl | BEGRENZT (Caveat: Zyklus → Endlosschleife, kein Stack-Risiko; Spec S-1) |
| 125 | `Type::root` | `root: Arc<Scheme>` (Refcount) | Konstruktor, kein Loop | BEGRENZT |
| 151 | `Type::child` | `self: Type` | Konstruktor | BEGRENZT |
| 157, 162 | `Type::resolved` | `self`, `bound: Type` | `while let TypKind::Var`, Kette ≤ Binderzahl | BEGRENZT (Caveat wie S-1) |
| 238, 243, 245 | `Scope::branch` | `condition: Type`, `self: Arc<Scope>`, `self` (None-Arm) | Konstruktor, pro Aufruf 1× | BEGRENZT |
| 339, 342, 349, 352 | `check` (Setup) | `typ: Type` (2×), `register.typ: Typ`, `checker.it: Arc` | einmalig pro `check()` | BEGRENZT |
| 399, 400, 413 | `Checker::resolve` | `synonym: Arc<Scheme>`, `typ.scope: Arc`, `typ: Type` | `while let Some(synonym)` über Abbrev-Kette; Kette ≤ Deklarationszahl, danach `pairs`-Stack | BEGRENZT (lange Abbrev-Ketten sind linear, nicht rekursiv) |
| 462, 465 | `open_value`/`value_type` | `typ: Type` (Return + Cache-Insert) | pro Aufruf 1× | BEGRENZT |
| 534 | `scalar_equal` | `s: Arc<Scheme>` (`Type::root`) | pro Aufruf 1× | BEGRENZT |
| 549, 551 | `annotation` | `typ: Typ` (AST-Knoten!), `scope: Arc` | pro Aufruf 1×; AST-`Typ`-Clone ist der teuerste hier (Baumtiefe), aber **nicht** stack-rekursiv (abgeleiteter Clone läuft auf normalem Stack; 10k-AST-Fall gehört Lane 16/19, Spec S-2) | BEGRENZT, Kosten-Caveat |
| 563 | `builtin` | AST-Knoten (`.clone()` in Builder-Kette) | pro Aufruf 1× | BEGRENZT |
| 573 | `id` | `typ: Type` (Return) | pro Aufruf 1× | BEGRENZT |
| 681 | `compatible` | `expected`, `found: Type` (Seed-`pairs`) | Seed des expliziten `pairs`-Stacks (`while let ... pop`, Z.682) | BEGRENZT |
| 734, 738 | `compatible` | `expected`/`found: Type` (`owner:`) | im `pairs`-Loop | BEGRENZT |
| 775 | `pattern` | `value: Typ` (AST, `Type::root`) | im `work`-Stack-Loop (Z.768); 1× pro Literal-Pattern | BEGRENZT |
| 799 | `pattern` | `typ: Type` (locals-Insert) | im `work`-Loop | BEGRENZT |
| 844, 871, 876, 880, 882 | `function` | `typ.root: Arc`, `scope: Arc` (2×), `result: Type` (2×) | Setup + `tasks`-Seeds (Z.878/885), pro Deklaration | BEGRENZT |
| 920, 976 | `infer` | `typ`/`result: Type` (Vec-Push, Cache-Insert) | im `work`-Stack-Loop (Z.916) | BEGRENZT |
| 1027, 1030 | `instantiate` | `typ: Typ` (Tupel-Map), `expected: Type` (Seed) | Seeds des `pairs`-Stacks (Z.1032) | BEGRENZT |
| 1096, 1101 | `instantiate` | `formal`/`actual: Type` (`owner:`) | im `pairs`-Loop | BEGRENZT |
| 1172 | `instantiate` | `s: Arc<Scheme>` | pro Aufruf 1× | BEGRENZT |
| 1204–1400 (19×: 1204, 1225, 1227, 1233, 1249, 1256, 1257, 1276, 1300, 1301, 1319, 1320, 1329, 1331, 1336, 1347, 1366, 1367, 1398, 1399, 1400 — gezählt 21 inkl. `callee.clone` 1276) | `run` | `scope: Arc` (7×), `returning: Type` (7×), `expected: Type` (3×), `callee: Type`, `bound: Type`, `element: Type` | alle im `tasks`-Loop (`while let Some(Task...)`, Z.1197); Stack ist Heap-`Vec` | BEGRENZT |

Exakte Zählung `run`: 1204, 1225, 1227, 1233, 1249, 1256, 1257, 1276, 1300,
1301, 1319, 1320, 1329, 1331, 1336, 1347, 1366, 1367, 1398, 1399, 1400.
`to_owned()`/`cloned()`-Rest (194, 578, 580, 788, 799, 841): String-
Allokationen bzw. `Option`-Lookups, alle BEGRENZT (Größe ≤ Bezeichnerlänge).

**Fazit canonical_body:** kein Handlungsbedarf für Lanes 16/19 aus
Stack-Sicht; offene Punkte nur als Specs S-1/S-2 (§6).

## 4. `program.rs` — alle 24 `.clone()`-Stellen (+ `to_vec`/`to_owned`)

Ebenfalls keine native Rekursion; `WellFormed::run` (Z.939, `while let ...
pop`) und `infer_pattern_type` (Z.707, `while let ... pop`) sind explizite
Stacks; `Drop for Types` (Z.699) ist bereits iterativ.

| Zeilen | umgebende fn | Was wird geklont | Tiefe | Urteil |
|---|---|---|---|---|
| 211 (`to_vec`), 213 (`to_vec`+`typ.clone`+`it.clone`) | `declare` | `quant: Vec<QuantItem>` (AST!), `typ: Typ` (AST!), `it: Arc` | 1× pro Typ-Alias-Deklaration; AST-Clones laufen auf normalem Stack | BEGRENZT (Kosten-Caveat wie S-2; 10k-Alias-Test besteht) |
| 252, 254, 282, 285, 292, 294, 295, 298 | `declare` | `Name` (Symbol-Copy), `it: Arc` (2×), `quant: Vec` (2×), `ctor.typ`/`ret: Typ` (AST) | 1× pro Deklaration | BEGRENZT |
| 355, 388, 391 | `declare` | `typschm: TypSchm`, `quant`, `ret` (AST) | 1× pro Val-/Funktions-Signatur | BEGRENZT |
| 411, 422 | `declare` | `reg.typ: Typ` (AST) | 1× pro Register | BEGRENZT |
| 433 | `declare` | `constraint: Constraint` (AST, in `Arc::new`) | 1× pro Constraint-Deklaration | BEGRENZT |
| 446 (`to_vec`) | `declare` | `overload`-Namen (`Vec<String>`) | 1× pro Overload-Gebrauch | BEGRENZT |
| 573 | `check_annotation` | `proof_vars: BTreeMap` (klein, prozedural) | pro Aufruf 1× | BEGRENZT |
| 639 | `overload_candidates` | `scheme: Arc<Scheme>` (Refcount) | pro Kandidat 1× | BEGRENZT |
| 710 | `Types::drop`-Helfer | `typ: Typ` (AST, in `types.0.push`) | im iterativen Drop-Stack | BEGRENZT |
| 809 | `quant` | `proof_vars` | pro Aufruf 1× | BEGRENZT |
| 973, 1089 | `run` | `signature: Arc<Constructor>` (Refcount, `Task::Discharge`) | im `work`-Stack | BEGRENZT |
| 996, 1100 | `run` | `proof_vars`, `variables` (kleine Maps) | pro `Assume`/`Restore`-Schritt | BEGRENZT (Kosten O(Variablen), keine Tiefe) |

`to_owned()`-Rest (Namens-Strings aus Interner, ~13 Stellen): alle
BEGRENZT (Größe ≤ Bezeichnerlänge, keine Tiefe).

**Fazit program:** kein Handlungsbedarf aus Stack-Sicht.

## 5. Was Lane-21 geändert hat (einzige Schreibstellen neben Report/Tests)

### 5a. `crates/sail-ty/src/conv.rs` — 4 gratuitous Deep-Clones entfernt

1. `typarg_nexp_only` (Z.385): `Option<NExp>` (klont ganzen NExp-Baum,
   selbst rekursiv!) → `Option<&NExp>` (Borrow); beide Aufrufer
   (`constraint_from_ast` Z.284, `typ_from_ast` Z.411) angepasst. Nebeneffekt:
   bestehende `unused_variables`-Warnung (`it`-Parameter) entfällt.
2. `typ_from_ast` (Z.413–427): `Bits`/`Range`-Arme klonten aus dem
   `nexps`-Vec (`n.clone()`, `lo/hi.clone()` — je ein kompletter rekursiver
   Baum-Clone) → **Moves** via `into_iter()` mit `len`-Guards. Akzeptierte
   Formen identisch (`bits` genau 1 Arg, `range` genau 2 Args, sonst
   `Named`-Fallback inkl. 0 Args); abgedeckt durch
   `typ_from_ast_moves_agree_with_clone_semantics`.
3. Verbleibende Clones (bewusst behalten): Z.797/Z.928 `keys.clone()`
   (`MiniExp::Config` ↔ `ExpKind::Config`, `Vec<String>`-Blatt ohne Tiefe;
   Größe ≤ Pragma-Schlüsselzahl aus der Quelle) — **BEGRENZT, Urteil: OK**.

### 5b. NEU `crates/sail-ty/src/recursion.rs` (+ 1 Zeile `pub mod` in `lib.rs`)

Iterative Depth-Guards mit explizitem Heap-Stack (nur Referenzen, kein Clone,
Short-Circuit bei Überschreitung), semantisch exakt `depth() > limit`:
`nexp_exceeds_depth`, `constraint_exceeds_depth`, `mini_typ_exceeds_depth`
(imitiert `check.rs`-Semantik `nexps()`+pro-Nexp-`depth`, ohne deren
rekursiven Clone), `typ_quant_exceeds_depth`. Unit-Tests: Boundary- und
Übereinstimmungsnachweis gegen rekursives `depth()` je Konstruktor.

### 5c. NEU `crates/sail-ty/tests/lane21_recursion.rs`

4 Tests (10k-Reject + Shallow-Accept auf 128 KiB, `typ_from_ast`-Move-
Äquivalenz, `NC_app`-Abstraktion). Sonden mit nativem `depth()`/`conv` auf
10k wurden **nicht** als Tests behalten (sie töten den Runner per SIGABRT;
Beleg ersatzweise in §2 protokolliert).

### 5d. Abnahme via Gate (`cargo max -j2`, `gated-run.sh`)

- `cargo test -p sail-ty --profile ci -j 2` (voll): **grün** —
  lib 33/33, `canonical_program` 35/35 (inkl. aller sieben 10k-Tiefentests:
  Vektor-Literale, Alias-Ketten, Konditional-Scopes, Tupel-Pattern,
  polymorphe Calls, Let-Scopes, abhängige Resultate), `typechecker` 13/13, `typechecker_full` 24/24,
  `lane21_recursion` 4/4, Rest (env/exp/prove/solver) grün.

## 6. Top-Risiken + exakte Ersetzungs-Specs (für nach Merge von Lane 16/19)

**R-1 (höchstes Risiko): `Nexp::depth` / `NConstraint::depth` (`nexp.rs:78, :215)
sind selbst das Leck.** Jeder `check.rs`-Guard (`check.rs:131, :430, :472`,
`CheckLimits::max_depth = 256`) ruft erst die rekursive Funktion und crasht
auf 10k-Eingaben, *bevor* er ablehnen kann (§2-Beleg). Gleiches gilt für alle
`conv.rs`-Konverter (§1, Ziff. 3), `MiniTyp::nexps` (`nexp.rs:391`, rekursiv
+ klont), `free_vars`/`as_const`/`as_bool`, alle `Display`-Impls
(`nexp.rs:155ff`, `write!("{a}")` rekursiert) und das abgeleitete `Drop`/`Clone`
der `Box`-Bäume.
**Spec D-1** (`nexp.rs`, nach Lane-16/19-Merge; Vorbild: neuer `recursion.rs`):

```diff
 // check.rs, drei Stellen (131, 430, 472): Guard ZUERST iterativ, dann erst ...
-if scheme.constraint.depth() > self.limits.max_depth {
+if crate::recursion::constraint_exceeds_depth(&scheme.constraint, self.limits.max_depth) {
     return Err(TypeError::fuel_exhausted(span));
 }
 // ... danach kann .depth() nur noch auf ≤256 tiefe Bäume treffen (harmlos).
```

**Spec D-2** (`conv.rs`, große iterative Rewrites — bewusst NICHT in Lane-21,
da nicht „klein/risikolos"): jede `*_to_ast`/`*_from_ast`-Funktion auf
expliziten `Vec`-Stack umstellen (Zustandsmaschine statt Call-Stack);
Eintritts-Guards `recursion::*_exceeds_depth(.., 256)` als Sofortschutz mit
`None`-Rückgabe (from-Seite) — to-Seite braucht dafür erst ein
fehlertolerantes Rückgabe-Schema (Lane-Entscheid).

**Spec S-1** (`canonical_body.rs:45-67, :156-165`, nach Lane-16-Merge):
`Scalar::resolved`/`Type::resolved`-Schleifen gegen Subst.-Zyklen härten
(`seen: BTreeSet<Symbol>`, bei Wiederholung `break` statt Endlosschleife).
Reiner Liveness-Fix, kein Stack-Thema.

**Spec S-2** (`canonical_body.rs:549, :563`, `program.rs:213f`):
AST-`Typ`-Clones (`annotation`, `builtin`, `declare`) auf `Arc<Typ>`-Sharing
oder Lazy-Normalisierung umstellen, falls Profiler (`perf/`) sie je als
heiß zeigen — korrekt, aber teuerste Clones im Audit.

**R-2 (Prio 2): abgeleitetes `Drop`/`Clone` für `Nexp`/`NConstraint`/
`MiniTyp`/`MiniExp`-Bäume.** Manuelles iteratives `Drop` (Vorbild:
`Substitutions::drop` in `canonical_body.rs:80-100`,
`sail_ast::storage::drop_definition`) oder `Arc`-Sharing; betrifft auch
Test-/Fehlerpfade, die tiefe Terme formatieren (`Display`, `TypeError`).

**R-3 (niedrig): `run`/`compatible`/`instantiate`-Clones** kopieren `Type`
(O(Arcs)) pro Work-Stack-Schritt — linearer Heap-Durchsatz, kein
Stack-Risiko; nur bei Profiler-Auffälligkeit anfassen.

## 7. Offene Behauptungen ohne Beleg — keine

Jede „BEGRENZT"-Einstufung stützt sich auf: (a) geprüfte Abwesenheit nativer
Rekursion in beiden Dateien, (b) sichtbare `while let ... pop()`-Stacks an
den angegebenen Zeilen, (c) grüne 10k-Tiefentests der bestehenden Suite via
Gate. Jede „UNBEGRENZT"-Einstufung stützt sich auf den §2-SIGABRT-Beleg.
Nicht behauptet wird: exakte Stack-Bytes pro Frame, Verhalten jenseits 10k,
Performance-Aussagen (keine Messung durchgeführt).

## 8. Re-Verifikation nach Host-Reboot (2026-10-12) + Merge-Hinweis Lane-11

**Basis unverändert:** Branch `lane-21` steht auf `a972e3a`, identisch mit
`main`-HEAD — die Abnahme wurde daher auf aktuellem Stand wiederholt
(kein Rebase nötig). `git status` im Worktree zeigt exakt die erwarteten
Einträge: `M conv.rs`, `M lib.rs`, `?? recursion.rs`,
`?? tests/lane21_recursion.rs`, `?? evidence/2026-10-12/`.

**Gate-Abnahme (neu, via `/tmp/opencode/sail-gate/gated-run.sh`):**
`cargo test -p sail-ty --profile ci -j 2` — **voll grün, 143/143, 0 failed:**
lib 33/33, `canonical_program` 35/35, `env_ast_bridge` 3/3,
`env_coverage` 5/5, `env_serde_roundtrip` 3/3, `exp_coverage` 17/17,
`lane21_recursion` 4/4, `prove_ctx` 3/3, `solver_parity` 3/3,
`typechecker` 13/13, `typechecker_full` 24/24. Entspricht exakt §5d.
Einzige Warnung (`unused import: Interner`) stammt aus Lane-21-fremden
Dateien (`prove_ctx.rs`/`env_ast_bridge.rs`/`ast_env.rs`/`advanced.rs`,
per grep verifiziert) — vorbestehend, nicht von Lane-21 verursacht.

**Lane-11-Überlappung (nur lesend geprüft, Haupt-Worktree nicht editiert):**
Der Haupt-Worktree enthält in `crates/sail-ty/src/conv.rs` einen reinen
Append-Block (+219 Zeilen ab Basis-Z.940, hinter `miniexp_from_ast`):
`TypeAlias`/`AliasEnv`/`expand_root`/`expand_rec`/`typ_from_mini_pat`/
`inline_signature_to_ast` + Tests. Lane-21 editiert ausschließlich
Basis-Z.281–427 (`constraint_from_ast`-Aufrufer, `typarg_nexp_only`,
`typ_from_ast`). **Textuell disjunkt → Merge konfliktfrei erwartet**
(reiner Append vs. Mid-File-Hunks). Semantische Vermerke für den Merge:

1. Lane-11 ruft `typarg_nexp_only` nirgends auf (grep im Haupt-Worktree:
   nur Z.289/390/416) — Lane-21s Signaturwechsel zu `Option<&NExp>` ist
   für Lane-11 transparent.
2. Lane-11s `expand_rec` (Haupt-Worktree `conv.rs:1066-1092) ist **nativ
   rekursiv** über `MiniTyp` (`Fun`/`Tuple`-Schachtelung + `Named`-Reentry)
   und `expand_root` klont pro Schritt `name`+`args` (Z.1047, Z.1060) —
   exakt die Musterklasse, die dieser Report als UNBEGRENZT bzw.
   gratuitous Clone einstuft. Der Report deckt sie nicht ab (Lane-11 ist
   jünger). Follow-up nach Merge: `expand_rec` auf expliziten Stack oder
   `recursion::*`-Guard am Eintritt; `expand_root`-Clones auf Borrows.
3. Lane-11s `inline_signature_to_ast` ruft `typ_to_ast` (rekursiver Konverter
   per §1, Ziff. 3) — erbt dessen UNBEGRENZT-Eigenschaft; ggf. ebenfalls
   per `recursion::mini_typ_exceeds_depth`-Guard sichern.

**Merge-bereit:** Lane-21 ist in sich abgeschlossen und abgenommen; beim
Merge mit Lane-11 sind keine manuellen Konfliktlösungen zu erwarten
(disjunkte Hunks), nur die obigen drei Follow-ups als neue Arbeit.
