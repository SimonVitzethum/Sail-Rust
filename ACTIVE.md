# Aktiv-Set

Integration (10.10.2026, Integrator, Welle 2): 14 Lanes nach main
gemergt (je ein Merge-Commit), Worktrees + Branches geloescht:
Lane 11, 17, 28, 29, 31–40. Lane 30 uebersprungen (uncommittete
Aenderungen, Spec QUEUED).

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
- Workspace: 40 Member (26 Bestand + 14 neue). Lane-15/lane-11-Stubs
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

Aktiv (noch in Arbeit, NICHT anfassen):
- Lane 30 structs-enums-unions (`worktrees/lane-30`,
  Branch lane/structs-enums-unions — uncommittete Aenderungen,
  Spec QUEUED, NICHT gemergt)
- Lane 41–58 (Branches lane/41-* ... lane/58-*, Worktrees lane-41..58)

Rotation: Lane fertig → mergen → Worktree löschen → nächsten Queued-Branch aktivieren:

```
./scripts/lane-rotate.sh <fertig-lane-nr> <neu-lane-nr>
# z.B. ./scripts/lane-rotate.sh 01 16
```

Regeln:
- Max. 15 Verzeichnisse in `worktrees/` — Skript bricht bei mehr ab.
- Jeder Merge braucht Lane-13-Diff-Report; RAM-Lanes brauchen Lane-14-Budget.
