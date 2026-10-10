# Frontend Diff Plan (Lane 86 — OCaml-Parität, E3: Code + Plan, kein Lauf)

Oracle: OCaml `sail` 0.20.3 (`/home/simon/.opam/sail/bin/sail`).
Fixtures: `sail/test/lean/*.sail` (36 Dateien, siehe `src/catalog.rs`).
Harness-Code: dieses Crate (`sail-frontend-diff`).

## 1. Oracle-Anrufungen (Rig-Lauf, später)

Pro Fall, `cwd` = gestagtes Scratch mit kargem Dateinamen (wie `sail-oracle`,
damit eingebettete Pfade upstream-identisch sind), unter
`ulimit -v <mem> + timeout <t>s`:

```text
sail --just-check --ddump-initial-ast <stem>.sail   # stdout -> Parse-Seite
sail --just-check --ddump-tc-ast <stem>.sail        # stdout -> Typ-Seite
```

`--just-check` bricht nach dem Typecheck ab (kein Lean-/Lake-Output nötig).
`$include <prelude.sail>` löst `sail` über sein Lib-Verzeichnis auf —
kein Zusatz-Flag nötig (zu verifizieren am ersten Rig-Lauf: falls
`prelude.sail` nicht gefunden wird, `--sail-lib`-/Env-Variante im Plan
nachtragen, kein stilles Skip).

## 2. Normalisierung (implementiert in `src/norm_tree.rs`)

| Quelle | Regel |
|---|---|
| Beide Seiten | `\r\n`/`\r` -> `\n`; Whitespace-Runs kollabiert; Banner (`Sail <version>`) + Leerzeilen-Dubletten raus (Oracle-Seite via `sail-oracle`-Prädikat) |
| Rust | Spans/NodeIds entfallen per Konstruktion; `file_start`/`file_end`-Shard-Marker gedroppt (gezählt, kein OCaml-Pendant) |
| Oracle | `DEF_<suffix>`-Token -> `NormKind`; Name = erster `"quoted"`-String dahinter; `l<num>[.c<num>]`-Fragmente entfernt |
| Header | Nur Kopftext (`sig_src`/`head_src`/Pragma-Payload); Bodies sind Skelett-Rohtext (Lane 02) und ausgenommen |
| Unbekannt | `DEF_<neu>` -> `Other` + Roh-Token in `head_norm` (knallt laut, kein stilles Pass) |

## 3. Vergleichsstufen

1. **Parse-Bäume** (`src/tree_compare.rs`): positionaler Vergleich
   Kind/Name/Header über die kanonische Sequenz; Längen-Tails als
   `MissingInActual`/`ExtraInActual`; Volltext-Diff via `sail-oracle`-Unified-Diff.
2. **Typ-Urteile, Stichprobe** (`src/ty_sample.rs`): deterministisch erste N=8
   `val`/`register`/`function`-Defs pro Datei; Rust-Seite = normalisierter
   Kopftext (ehrliches Skelett-Rendering — volle `sail-ty`-Elaboration pro
   Sample ist Lane-05+-Folgearbeit), Oracle-Seite = `sift_tc_dump`-Paare.
   Duplikate auf einer Seite = Mismatch (Sifter-Bug-Signal).

## 4. Fall-Katalog (`src/catalog.rs`)

- **Smoke (8)**: `trivial`, `enum`, `let`, `registers`, `tuples`,
  `type_kid`, `undefined`, `union` — winzige Dateien über Kern-Def-Arten.
- **Standard (25)**: alles ≤ 8192 Bytes inkl. `errors` (ParseOnly —
  Oracle-Typecheck scheitert erwartbar, daher kein tc-Dump).
- **Nightly (3)**: `SailTinyArm`, `SailTinyArmV2`, `map_tactics` (> Cap).

## 5. Rig-Lauf (Schritte, später)

1. `scripts/limited-run.sh` + tmux nach Lane 51 (MemoryMax beachten).
2. Erst `--only trivial,let,registers` (Sifter-Verifikation!): Dump-Format
   gegen `sift_oracle_dump`/`sift_tc_dump`-Annahmen prüfen; bei Drift Sifter
   fixen (HarnessError-Pfad nutzen, kein Aufweichen der Normalisierung).
3. Dann Smoke, dann Standard, Nightly separat.
4. Report: `FrontendResult`-Liste mit Status+Detail (Markdown-Renderer ist
   Folgearbeit; bis dahin `reason()`-Log + Diff-Details).

## 6. Akzeptanz

- Smoke: alle Parse-Vergleiche `ParseMatch` (nach Sifter-Verifikation).
- Standard: jede Abweichung als `ParseMismatch`/`TySampleMismatch` mit Diff
  triagiert; `errors` = ParseOnly-Skip mit Begründung.
- Kein `HarnessError` offen; Katalog-Byte-Test grün (Fixture-Drift erkannt).
- Bekannte Skelett-Lücken (Bodies, `Type`/`Let`-Bridge-`deferred`) als
  Mismatches dokumentiert, nicht normalisiert-weggelaugt.

## 7. Risiken

- Dump-Zeilenformate unverifiziert (E3: keine Läufe) — Erst-Rig-Lauf ist
  Sifter-Validierung, kein Produktivlauf.
- Rust-Header vs. OCaml-Dump-Header sind verschiedene Sprachen (Sail-Quelltext
  vs. OCaml-Pretty-Print): `HeadMismatch`-Rate initial hoch möglich; Eskalation
  = Header-Vergleich auf Kind+Name reduzieren (bewusst, dokumentiert), nicht
  stillschweigend.
- `guardedmatch` hat Sibling-`.json` (Staging kopiert es, wie `sail-oracle`).
