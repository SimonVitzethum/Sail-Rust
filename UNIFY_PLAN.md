# UNIFY_PLAN.md — Lane 48 workspace-unify (Analyse + Plan, kein Umbau)

Stand: 10.10.2026, Branch `lane/48-workspace-unify`, HEAD `874a24c`.
E3 gilt: **kein `cargo`/Build/Test durch Lane-Agents** (OOM-Schutz, kein Swap).
Verifikation (Gates) läuft **ausschließlich beim Integrator** mit Memory-Limits,
sequenziell, nicht parallel über ~40 Worktrees.

Dieser Plan ist **nur Analyse + Reihenfolge**. Er schreibt nichts um.
Der Integrator arbeitet ihn von oben nach unten ab; jeder Schritt nennt
**behalten / verwerfen** und die exakte Merge-Reihenfolge.

---

## 1. Workspace-Ist (HEAD, = main nach Integrator-Merge von 25 Lanes)

- Root-`Cargo.toml`: 26 Member (`sail-ast`, `-parse`, `-resolve`, `-nexp`,
  `-ty`, `-solver`, `-rewrite`, `-effects`, `-lower`, `-lean`, `-lake`,
  `-oracle`, `-perf`, `-shard`, `-cli`, `-lex`, `-prelude`, `-regs`,
  `-mapping`, `-patterns`, `-bitvec`, `-int`, `-real`, `-extern`,
  `-termination`, `-loops`), `resolver = "2"`.
- `[workspace.package]`: edition 2021, rust-version 1.75.0 (MSRV-Pin),
  license MIT OR Apache-2.0, version 0.0.0 — **behalten**.
- `[workspace.lints]`: `rust.unsafe_code = "forbid"`,
  `clippy.all = "warn"`, `clippy.pedantic = "allow"` — **behalten**.
- Geprüft (grep, kein cargo): alle 26 `crates/*/Cargo.toml` haben
  `[lints] workspace = true`, alle 26 Crate-Roots (`lib.rs`/`main.rs`)
  haben `#![forbid(unsafe_code)]`. **Kein fehlendes `[lints]` im Ist.**
- Lane-15-Erbe **behalten**: `.github/workflows/ci.yml`,
  `scripts/check.sh`, `docs/branch-protection.md`,
  `.gitignore` (`worktrees/`, `target/`, `Cargo.lock` — Lock/target nie tracken).

### 1.1 Lane-15-Stub-Reste (2, beide zu ersetzen, nicht zu behalten)

| Stub | Befund | Ersatz (echt gewinnt) |
|---|---|---|
| `crates/sail-shard/src/lib.rs` | reiner Platzhalter (`Platzhalter (Lane 15)`, nur leerer Test) | Lane 11 (uncommitted im Worktree `lane-11`: `shard.rs`, `fingerprint.rs`, `scheduler.rs`, `incremental.rs`) — **Stub verwerfen, Lane-11-Code übernehmen** |
| `crates/sail-cli/src/main.rs` | Platzhalter (`noch nicht implementiert`, `exit(2)`) | Lane 38 `lane/cli-frontend` (1 Commit `f5a3ab9`: `args.rs`, `main.rs`, `memo.rs`, `profile.rs`, `tests/gen_lean_dropin.rs`) — **Stub verwerfen, Lane-38-Code übernehmen** |

### 1.2 Offene Swap-Verträge (gehören Lane 47, blockieren aber Duplikat-Bereinigung)

HEAD enthält 5 `*_NEEDED.md` (in allen Post-Integrator-Branches 41–51 identisch):
`LEXER_NEEDED.md`, `AST_NEEDED.md` (in `sail-parse/`), `REGS_NEEDED.md`,
`RESOLVE_NEEDED.md`, `TY_NEEDED.md`. Dazu branch-lokal:
`SHARD_NEEDED.md` (Lane 31), `CACHE_NEEDED.md` (Lane 36), `docs/CLI_NEEDED.md`
(Lane 40). **Regel: Lane 48 löst keine Minimaltypen auf** (das ist Lane 47),
sondern übernimmt sie unverändert und vermerkt pro Crate den Vertrag
(siehe §3). Integrator stimmt sich mit Lane 47 ab: `sail-typedef` (Lane 30,
§3.13) und alle `mini_ast`/`MinimalDef`-Duplikate fallen unter Lane 47.

---

## 2. Branch-Landschaft (Befund per `git show`/`merge-base`, kein Checkout)

- **Alt-Branches** (Basis `b74415f`, HEAD ist 105 Commits weiter, je ~1 eigener
  Commit, **minimaler Workspace mit 1–2 Membern**): 13 neue Crates + Lane 17/11.
  Beim Merge je Branch: **Root-`Cargo.toml` der Lane verwerfen**
  (Minimal-`members` + ggf. abweichende `[workspace.*]`), **nur `crates/<neu>/`
  übernehmen** und Member-Zeile in den HEAD-Workspace eintragen.
- **Post-Integrator-Branches** (Basis = HEAD): 41–51, Specs/Doku, keine
  Cargo-Konflikte. `lane/structs-enums-unions` = Sonderfall (Branch ohne eigene
  Commits, echte Arbeit uncommitted im Worktree `lane-30`).
- **Uncommitted-Arbeit** (muss VOR dem Unify committet sein, sonst geht sie
  beim Worktree-Löschen verloren): `lane-11` (echtes `sail-shard`),
  `lane-17` (6 geänderte `sail-grammar`-Dateien + neue `tests/rules.rs`),
  `lane-30` (`sail-typedef` + Member-Zeile), `lane-32` (3 Dateien),
  `lane-43` (`crates/sail-sched/`), `lane-46` (`perf/arm-v8.5-a-plan.md`),
  `lane-51` (`[profile.ci]` + `logs/` + 4 neue Dateien), `lane-29`
  (`Cargo.lock` untracked — **löschen, nie tracken**).

---

## 3. Einträge je Lane (je: behalten / verwerfen / Fix vor Merge)

Legende Fix-Kürzel: **L** = `[lints] workspace = true` setzen,
**W** = `version/edition/rust-version/license.workspace = true` (statt hartem
`0.1.0`), **F** = `#![forbid(unsafe_code)]` im Root prüfen,
**M** = Member-Zeile in Root-Workspace eintragen, Root-`Cargo.toml` der Lane
verwerfen.

### 3.1 Lane 11 `lane/11-sharding-incremental` — sail-shard (echt)
- **Behalten:** `crates/sail-shard/` aus Worktree `lane-11` (Branch selbst ist
  leer, Arbeit uncommitted: `Cargo.toml`-Diff `members = ["crates/sail-shard"]`
  + `shard.rs`/`fingerprint.rs`/`scheduler.rs`/`incremental.rs`).
- **Verwerfen:** HEAD-Stub `crates/sail-shard/src/lib.rs`, Lane-Root-`Cargo.toml`.
- Fix: **W, L, F** am Worktree-Stand prüfen (dort `version = "0.1.0"` hart +
  neue Deps `blake3`, `crossbeam-channel`, `rayon` — Integrator prüft
  Lizenzen + `geiger`-Sauberkeit dieser Deps). **M**.
- Merge-Position: **#1** (Stub-Ablösung; `SHARD_NEEDED`/`CACHE_NEEDED`-Verträge
  von Lane 31/36 beziehen sich darauf, Migration aber erst nach Lane 47).

### 3.2 Lane 17 `lane/grammar-menhir-port` — sail-grammar
- **Behalten:** `crates/sail-grammar/` (1 Commit `24f727f` + uncommitted Fixes
  im Worktree; erst die uncommitted Änderungen committen lassen).
- **Verwerfen:** Lane-Root-`Cargo.toml` (`members = ["crates/sail-grammar"]`).
- Fix: **L** (ist `[lints.rust]`, muss `[lints] workspace = true`),
  **W**, Lizenz `BSD-2-Clause` → `MIT OR Apache-2.0` (Workspace-Standard,
  mit Lane-17-Owner klären; sonst Ausnahmegrund dokumentieren),
  `build.rs`/LALRPOP-Artefakt unverändert übernehmen. **M**.

### 3.3 Lane 28 `lane/early-return-except` — sail-except
- **Behalten:** `crates/sail-except/` (Commit `d6094da`, sauber, keine
  uncommitted Reste).
- **Verwerfen:** Lane-Root-`Cargo.toml`.
- Fix: **L** (hat **gar keine** `[lints]`-Sektion — nachholen), **W**, **F**
  (vorhanden, bestätigen). **M**.

### 3.4 Lane 29 `lane/concurrency-archsem` — sail-archsem
- **Behalten:** `crates/sail-archsem/`.
- **Verwerfen:** Lane-Root-`Cargo.toml` (vollständiger `[workspace.*]`-Block,
  wird nicht gebraucht); untracked `Cargo.lock` im Worktree **löschen**.
- Fix: **W** (`version = "0.1.0"` hart), L ist ok. **M**.

### 3.5 Lane 30 `lane/structs-enums-unions` — sail-typedef (Sonderfall)
- **Behalten:** `crates/sail-typedef/` + 1 Member-Zeile aus Worktree `lane-30`
  (Branch hat keine eigenen Commits; Arbeit uncommitted: 8 `src/`-Dateien,
  `tests/difftest.rs`, L = ok).
- **Verwerfen:** nichts im Branch (kein Delta); HEAD-`sail-ast` bleibt Master.
- Fix: **W** (`version = "0.1.0"` hart), **M** (Zeile
  `"crates/sail-typedef",` hinter `sail-loops` einfügen).
- **Lane-47-Abstimmung Pflicht:** `sail-typedef` vs. `sail-ast`-Typfamilie —
  vorerst als eigenes Member übernehmen, Auflösung der Doppel-Typen
  (`typedef.rs`/`types.rs` vs. `sail-ast/src/ast.rs`) an Lane 47 übergeben,
  hier nichts löschen.

### 3.6 Lane 31 `lane/armv9-sysregs-shard` — sail-sysregs-shard
- **Behalten:** `crates/sail-sysregs-shard/` (bin-Crate, `SHARD_NEEDED.md`
  als Vertrag mit Lane 11 mitübernehmen, nicht löschen).
- **Verwerfen:** Lane-Root-`Cargo.toml`.
- Fix: **L** (ist `[lints] rust.unsafe_code`, muss `workspace = true`),
  **W** (alles hart gepinnt), **M**. Name `sail-sysregs-shard` bleibt
  (kein Konflikt mit `sail-shard`).

### 3.7 Lane 32 `lane/armv9-sve-sme-shard` — sail-sve-sme
- **Behalten:** `crates/sail-sve-sme/` inkl. uncommitted Fixes im Worktree
  (`patterns.rs`, `FAILURES_SVE_SME.md` — erst committen lassen).
- **Verwerfen:** Lane-Root-`Cargo.toml`.
- Fix: **W**, L ok. **M**.

### 3.8 Lane 33 `lane/armv9-instrs-core-shard` — sail-instrs-core
- **Behalten:** `crates/sail-instrs-core/` (vorbildlich: W + L bereits ok).
- **Verwerfen:** Lane-Root-`Cargo.toml`. Fix: nur **M**.

### 3.9 Lane 34 `lane/elfmain-executable` — sail-exe
- **Behalten:** `crates/sail-exe/`.
- **Verwerfen:** Lane-Root-`Cargo.toml`.
- Fix: **W**, L ok, **Pfadfix Pflicht**:
  `[dev-dependencies] sail-lake = { path = "../../../../crates/sail-lake" }`
  → `{ path = "../sail-lake" }` (nach Unify Nachbar im selben Workspace;
  `../../../../` ist Relikt des Minimal-Workspaces und bricht). **M**.

### 3.10 Lane 35 `lane/error-diagnostics` — sail-diag
- **Behalten:** `crates/sail-diag/`. **Verwerfen:** Lane-Root-`Cargo.toml`.
- Fix: **W**, L ok. **M**. Hinweis: als künftiger Shared-Fehlertyp
  dokumentieren, aber keine bestehenden Fehlertypen anderer Crates migrieren
  (kein Umbau in Lane 48).

### 3.11 Lane 36 `lane/incremental-cache-disk` — sail-cache
- **Behalten:** `crates/sail-cache/` + `CACHE_NEEDED.md` (Vertrag mit Lane 11).
- **Verwerfen:** Lane-Root-`Cargo.toml`. Fix: nur **M** (W + L bereits ok).

### 3.12 Lane 37 `lane/arena-interner-tuning` — sail-arena (+ sail-ast-Kopie)
- **Behalten:** `crates/sail-arena/` (L ok, `sail-ast`-Dep als Pfad-Dep
  `../sail-ast` → nach Unify auf Workspace-Dep umstellen oder Pfad lassen
  wenn relativ stabil — Integrator entscheidet, dokumentiert).
- **Verwerfen:** mitgebrachte `crates/sail-ast/`-Kopie (per Diff identisch
  mit HEAD — **Dedup: Kopie verwerfen, nur `sail-arena` übernehmen**),
  Lane-Root-`Cargo.toml` (`members = ["crates/sail-arena", "crates/sail-ast"]`
  → nur Arena wandert in den Haupt-Workspace).
- Fix: **W** (`version = "0.1.0"` hart), **M**.

### 3.13 Lane 38 `lane/cli-frontend` — sail-cli (echt)
- **Behalten:** `crates/sail-cli/` komplett (Commit `f5a3ab9`: `args.rs`,
  `main.rs`, `memo.rs`, `profile.rs`, `tests/gen_lean_dropin.rs`, `README.md`).
- **Verwerfen:** HEAD-Stub-`main.rs` (echt gewinnt), Lane-Root-`Cargo.toml`
  (dort `version = "0.1.0"` unter `[workspace.package]` — nicht übernehmen).
- Fix: W ist im Crate bereits ok (`version.workspace` etc.), L ok. Member-Zeile
  existiert bereits — nur Inhalt ersetzen. Merge-Position: **#2**.

### 3.14 Lane 39 `lane/c-parity-smoke` — sail-smoke
- **Behalten:** `crates/sail-smoke/`. **Verwerfen:** Lane-Root-`Cargo.toml`.
- Fix: **W**, L ok. **M**.

### 3.15 Lane 40 `lane/release-packaging` — sail-release-meta
- **Behalten:** `crates/sail-release-meta/` + `docs/CLI_NEEDED.md`
  (mitübernehmen, nicht löschen).
- **Verwerfen:** Lane-Root-`Cargo.toml`.
- Fix: **L** (ist `[lints] rust.unsafe_code`, muss `workspace = true`),
  **W** (hart gepinnt), **M**.

### 3.16 Nachzügler (uncommitted, NICHT Teil des Unify-Merges — nur vormerken)
- `lane-43`-Worktree `crates/sail-sched/` (`config.rs`, `lib.rs`), `lane-46`
  `perf/arm-v8.5-a-plan.md`, `lane-51` (`[profile.ci]`-Block,
  `.gitignore`-Zeile `logs/`, `docs/TEST-RIG.md`, `lanes/51-test-rig.md`,
  `scripts/limited-run.sh`, `scripts/tmux-test.sh`): **nicht in Lane 48
  einbauen** — jeweiligen Owners zum Committen auffordern; `[profile.ci]` +
  `logs/`-Ignore sind legitime Unify-Kandidaten für einen Folge-Commit,
  aber kein Blocker.

---

## 4. Ausführungsreihenfolge für den Integrator (Copy-paste-fähig)

**Phase 0 — Commits einholen (Voraussetzung, kein Merge ohne):**
1. Owner Lane 11: Worktree-Stand committen (`sail-shard` echt + Deps).
2. Owner Lane 17: uncommitted `sail-grammar`-Änderungen + `tests/rules.rs` committen.
3. Owner Lane 30: `sail-typedef` + Member-Zeile committen.
4. Owner Lane 32: `patterns.rs` + `FAILURES_SVE_SME.md` committen.
5. `lane-29`-Worktree: untracked `Cargo.lock` löschen (nie tracken).

**Phase 1 — Stub-Ablösung (Reihenfolge bindend):**
6. Lane 11 mergen: `crates/sail-shard/` übernehmen (§3.1), Stub verwerfen,
   Member-Zeile bleibt (nur Inhalt wechselt).
7. Lane 38 mergen: `crates/sail-cli/` übernehmen (§3.13), Stub-`main.rs`
   verwerfen, Member-Zeile bleibt.

**Phase 2 — Neue Crates (konfliktarm, Lane-Nummern-Reihenfolge):**
8. Lane 17 (§3.2) → 28 (§3.3) → 29 (§3.4) → 31 (§3.6) → 32 (§3.7) →
   33 (§3.8) → 34 (§3.9, mit Pfadfix) → 35 (§3.10) → 36 (§3.11) →
   37 (§3.12, ohne `sail-ast`-Kopie) → 39 (§3.14) → 40 (§3.15).
9. Lane 30 (§3.5) zuletzt (wegen Lane-47-Abstimmung).
10. Pro Crate nach Merge: Fix-Checkliste L/W/F/M aus §3 abhaken, Member-Zeile
    ans Ende der `members`-Liste (hinter `sail-loops`, `sail-typedef` ganz
    zuletzt), Root-`Cargo.toml`-`[workspace.*]` der Lane nie übernehmen.

**Phase 3 — Gates (nur Integrator, sequenziell, mit Memory-Limits):**
11. `grep -rn "\[lints\]" crates/*/Cargo.toml` → überall `workspace = true`;
    `grep -rn "forbid(unsafe_code)" crates/*/src/` → überall vorhanden.
12. `scripts/check.sh` (fmt + clippy + unsafe-Grep) grün.
13. `cargo geiger --forbid-only` sauber (inkl. neuer Deps `blake3`,
    `crossbeam-channel`, `rayon`); `cargo test --workspace` grün.
    Erst bei grünem Gate ist Lane 48 Done (Spec §Done).

**Nicht tun:** keine Minimaltypen auflösen (`*_NEEDED.md` bleiben bis Lane 47),
keine Fehlertypen vereinheitlichen, keine `Cargo.lock`/`target/` committen,
keine parallelen `cargo`-Läufe (E3).

---

## 5. Erwartetes Ergebnis

- Ein Workspace mit 26 + 14 = **40 Membern** (26 Ist + `sail-grammar`,
  `sail-except`, `sail-archsem`, `sail-sysregs-shard`, `sail-sve-sme`,
  `sail-instrs-core`, `sail-exe`, `sail-diag`, `sail-cache`, `sail-arena`,
  `sail-smoke`, `sail-release-meta`, `sail-typedef`, `sail-cli`/`sail-shard`
  bleiben Zeilen, wechseln nur Inhalt).
- Keine toten Member, keine doppelten Minimal-Workspaces, keine Stub-Reste,
  `[lints] workspace = true` überall, `*_NEEDED.md`-Verträge intakt für Lane 47.
