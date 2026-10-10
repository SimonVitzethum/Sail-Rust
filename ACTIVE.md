# Aktiv-Set

Integration (10.10.2026, Integrator): 25 Lanes nach main gemergt (je ein
Merge-Commit), Worktrees + Branches geloescht.

DONE (gemergt in main, Worktree+Branch geloescht):
Lane 01–10, 12–16, 18–27.
- 01 ast-core, 02 parser, 03 name-resolve, 04 nexp-constraint,
  05 typechecker, 06 z3-bridge-memo, 07 rewrites-core,
  08 callgraph-effects, 09 mono-anf-lowering, 10 lean-printer-stream,
  12 lake-project-gen, 13 test-oracle-diff, 14 perf-ram-profiling,
  15 ci-forbid-unsafe, 16 lexer,
  18 prelude-lib, 19 register-model, 20 mapping-encdec,
  21 pattern-exhaustiveness, 22 bitvector-semantics,
  23 int-arith-semantics, 24 real-support, 25 extern-ffi-mapping,
  26 termination-measures, 27 loops.

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

Aktiv (noch in Arbeit, NICHT anfassen):
- Lane 11 sharding-incremental (`worktrees/lane-11`,
  Branch lane/11-sharding-incremental)
- Lane 17 grammar-menhir-port (`worktrees/lane-17`,
  Branch lane/grammar-menhir-port)

Rotations-Pool (Spec + Branch, Worktrees lane-28..40 unberuehrt):
Lane 28–40 (early-return-except, concurrency-archsem, structs-enums-unions,
armv9-sysregs-shard, armv9-sve-sme-shard, armv9-instrs-core-shard,
elfmain-executable, error-diagnostics, incremental-cache-disk,
arena-interner-tuning, cli-frontend, c-parity-smoke, release-packaging).

Rotation: Lane fertig → mergen → Worktree löschen → nächsten Queued-Branch aktivieren:

```
./scripts/lane-rotate.sh <fertig-lane-nr> <neu-lane-nr>
# z.B. ./scripts/lane-rotate.sh 01 16
```

Regeln:
- Max. 15 Verzeichnisse in `worktrees/` — Skript bricht bei mehr ab.
- Jeder Merge braucht Lane-13-Diff-Report; RAM-Lanes brauchen Lane-14-Budget.
