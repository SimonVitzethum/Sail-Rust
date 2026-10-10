# Aktiv-Set

Integration (10.10.2026, Integrator, Welle 3): 15 Lanes nach main
gemergt (je ein Merge-Commit mit -s + Co-authored-by-Trailer),
Worktrees + Branches geloescht:
Lane 30, 32f, 45–57. Keine Lane uebersprungen (alle 15 vorhanden).

DONE (gemergt in main, Worktree+Branch geloescht):
Lane 01–11, 12–17, 18–29, 31–40.
- 01 ast-core, 02 parser, 03 name-resolve, 04 nexp-constraint,
  05 typechecker, 06 z3-bridge-memo, 07 rewrites-core,
  08 callgraph-effects, 09 mono-anf-lowering, 10 lean-printer-stream,
  11 sharding-incremental, 12 lake-project-gen, 13 test-oracle-diff,
  14 perf-ram-profiling, 15 ci-forbid-unsafe, 16 lexer,
  17 grammar-menhir-port,
  18 prelude-lib, 19 register-model, 20 mapping-encdec,
  21 pattern-exhaustiveness, 22 bitvector-semantics,
  23 int-arith-semantics, 24 real-support, 25 extern-ffi-mapping,
  26 termination-measures, 27 loops,
  28 early-return-except, 29 concurrency-archsem,
  31 armv9-sysregs-shard, 32 armv9-sve-sme-shard,
  33 armv9-instrs-core-shard, 34 elfmain-executable,
  35 error-diagnostics, 36 incremental-cache-disk,
  37 arena-interner-tuning, 38 cli-frontend, 39 c-parity-smoke,
  40 release-packaging.
  30 structs-enums-unions (sail-typedef doc_typedef-Port + Soundness-Fix
  implies_nneg/RegisterRef-Trennzeichen), 32f sve-fix (tokenize-Loop,
  throw-Ausnahme, Test-Label, Ctx.src raus),
  45 e2e-tiny-arm (sail-cli-e2e-Modul + Fail-Liste),
  46 e2e-armv85 (Durchlaufplan + Messgeruest), 47 ast-unify
  (MIGRATION.md, Analyse), 48 workspace-unify (UNIFY_PLAN.md, Analyse only),
  49 ci-gates (E2-/Unsafe-/Budget-Gates, Budget warnend),
  50 smp-boot (Abnahme-Doku + Harness-Skizze),
  51 test-rig (systemd-run + tmux + Spec), 52 sorry-elim (sail-regs
  harter E3512-Fehler mit Span), 53 arch-docs (ARCHITECTURE.md),
  54 gabbro-survey (SURVEY.md), 55 budget-consol (BUDGETS_ALL.md),
  56 wave3-prep (WAVE3.md Mergeplan + Gate), 57 lean-contract
  (sail-lake Golden-Contract + Layout-Register).
- Workspace: 41 Member (40 Bestand + sail-typedef neu). Lane-15/lane-11-Stubs
  ersetzt wo echte Crates existieren (echt gewinnt: sail-shard Lane 11,
  sail-cli Lane 38).

Merge-Notizen:
- Root-Cargo.toml: Workspace-Member aller echten Crates vereint (26 Member
  inkl. sail-shard/sail-cli-Stubs aus Lane 15) + Lane-15-Metadaten
  ([workspace.package], [workspace.lints], MSRV 1.75.0).
- Lane-15-Stubs verworfen wo echte Crates existieren (echt gewinnt);
  behalten aus Lane 15: .github/workflows/ci.yml, scripts/check.sh,
  docs/branch-protection.md, sail-cli/sail-shard-Stubs, Workspace-Lints
  ([lints] workspace=true in allen Crates).
- .gitignore: worktrees/ + target/ + Cargo.lock; Cargo.lock/target nie
  getrackt (Lane-15-target-Ballast + Lockfiles aus Merges entfernt).
- Verifikation: cargo test --workspace 395 passed / 0 failed;
  forbid(unsafe_code) in allen Crate-Roots, kein unsafe-Token im Code
  (nur Doc-Kommentar-Erwahnungen).

Welle-2-Notizen (E3, Test-Moratorium — KEINE Builds, Review-only):
- Echte Lane-Crates behalten (echt gewinnt ueber Stub); sail-shard- und
  sail-cli-Cargo.toml auf Workspace-Metadaten + [lints] workspace=true
  angeglichen (Lane-11-Branch war aelter als die Lane-15-Konvention).
- .gitignore: main-Stand behalten (worktrees/ + target/ + Cargo.lock);
  Cargo.lock nirgends getrackt; alte worktrees/-Gitlinks aus Lane-Branches
  nicht uebernommen.
- Root-Cargo.toml: [profile.release] (lto thin, strip, panic unwind) +
  [profile.ci] (inherits release, lto false, debug false) fuer
  speicherschonende Test-Binaries.
- Neu: .cargo/config.toml ([build] jobs = 4, RAM-Schutz, gilt fuer alle
  Worktrees darunter).
- Alle worktrees/*/target/ geloescht (~1,6 GB Plattenplatz; kein Agent
  baut per E3).
- Review-Check Welle 2: forbid(unsafe_code) in allen neuen Crate-Roots,
  kein unsafe-Token im Lane-Code. Build/Test-Verifikation offen
  (Test-Rig der Haupt-Session).

Welle-3-Notizen (E3, Test-Moratorium — KEINE Builds, Review-only):
- Lane 45 (Konflikt main.rs): Lane-38-Voll-CLI behalten (echt), Lane-45-
  Treiber als `mod e2e` verdrahtet statt main.rs-Ersatz; e2e.rs + Cargo-Deps
  + Fail-Liste uebernommen. forbid(unsafe_code) im CLI-Root erhalten.
- Lane 51 (Konflikt Root-Cargo.toml): HEAD-Profile behalten ([profile.release]
  + [profile.ci] inherits release); Lane-51-[profile.ci]-Anhang (inherits dev)
  als Doppelkey entfallen, Substanz bereits abgedeckt. .gitignore vereint
  (logs/-Zeile uebernommen).
- Lane 30 (Konflikt Root-Cargo.toml): Member-Union (sail-typedef nach
  sail-loops einsortiert, alle HEAD-Member behalten). Crate mit [lints]
  workspace=true + forbid im Root.
- Lane 52: keine externen Aufrufer von initial_regstate_snippet ausser
  sail-regs selbst (mitmigriert).
- UNIFY_PLAN (Lane 48) + MIGRATION.md (Lane 47) nur gelesen/uebernommen,
  NICHT umgesetzt (Umsetzung Lane-47/48-Sache nach Merge).
- Cargo.lock nirgends getrackt; keine Konfliktmarker; unsafe-Treffer nur
  Doc-Kommentare; forbid(unsafe_code) in allen 41 Crate-Roots.
- Build/Test-Verifikation offen (Test-Rig der Haupt-Session).

E3-Integration (10.10.2026, Integrator — KEINE Builds, nur Merges):
9 Merge-Commits (je --no-ff, -s + Co-authored-by-Trailer), Branches +
Worktrees geloescht: Lane 62, 63, 64, 65, 66, 67, 69, 70, 58 (selektiv).
Uebersprungen + Grund:
- lane/structs-enums-unions (Lane 30): Branch existiert nicht — nichts zu mergen.
- lane/41-lang-delta, lane/42-shared-mem, lane/43-scheduler-replay,
  lane/44-el3-psci-boot: gesperrt (Gate: Port-durch+lauft+RAM-opt) — NICHT angefasst.
- lane/68-ramopt-impl: nicht im Auftrag — NICHT angefasst.
- lane/58-ram-opt: selektiv (nur RAM_ANALYSIS.md, fehlte auf Main); Rest des
  Branches veraltet (Basis b079704, haette Main-Arbeit zurueckgesetzt).
Merge-Notizen:
- Lane 63 (Konflikt AST_NEEDED.md): Lane-61-Statusblock (HEAD) + Lane-63-
  Bridge-Stand behalten (disjunkte Doku-Abschnitte).
- Lane 64 (Konflikt sail-regs lib.rs-Dok): bridge-Modul (echt) + Lane-61-
  Retain-Kontext vereint.
- Lane 65 (Konflikte sail-ty Cargo.toml/env.rs/kind.rs): Union — sail-ast-Dep
  (HEAD) + serde-Pins (Branch); Retain-Docs (HEAD) + Serialize/Deserialize-
  Derives (Branch).
- Lane 70 (sail-parse/Cargo.toml): von Hand vereint — Auto-Merge hatte
  [lib]/[dependencies] gedoppelt (invalides TOML, aus Lane-63-Merge
  verschleppt); alle 42 Manifeste per TOML-Parse validiert.
- lanes/-Notizen 64/65/66/67/70 (uncommitted in Worktrees) nach lanes/
  uebernommen (62/63/69 kamen per Branch-Commit).
- Checks je Merge: kein Cargo.lock getrackt, kein unsafe-Token ausser
  forbid-Zeilen, keine Konfliktmarker. Build-Verifikation offen (Rig).

Aktiv (noch in Arbeit, NICHT anfassen):
- Lane 41–44 (Branches lane/41-lang-delta, lane/42-shared-mem,
  lane/43-scheduler-replay, lane/44-el3-psci-boot, Worktrees lane-41..44 —
  BLOCKED bis Gate: Port durch + laeuft + RAM-opt, siehe WAVE3.md)
- Lane 58: RAM_ANALYSIS.md per Selektiv-Merge auf Main (Branch + Worktree
  geloescht); weiterfuehrende RAM-Opt erst nach Unify (selber Gate wie oben)
  sowie lane/59-grammar-gen (ausserhalb Welle 3)

Rotation: Lane fertig → mergen → Worktree löschen → nächsten Queued-Branch aktivieren:

```
./scripts/lane-rotate.sh <fertig-lane-nr> <neu-lane-nr>
# z.B. ./scripts/lane-rotate.sh 01 16
```

Regeln:
- Max. 15 Verzeichnisse in `worktrees/` — Skript bricht bei mehr ab.
- Jeder Merge braucht Lane-13-Diff-Report; RAM-Lanes brauchen Lane-14-Budget.
