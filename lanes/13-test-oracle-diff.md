# Lane 13 — test-oracle-diff
Branch: `lane/13-oracle-diff` | Worktree: `sail-rs/worktrees/lane-13` (nach Merge löschen)

## Ziel
Differentielle Tests gegen OCaml-Sail als Orakel.

## Aufgaben
- Harness: für jedes `sail/test/lean/*.sail` OCaml-Output vs. Rust-Output diffen; ARM-Subset (`aarch64_small`, `arm-v8.5-a`-Ausschnitt) als Nightly.
- Failing-Liste mit Span + Rewrite-Pass pflegen, kein Wegschauen.

## Done
- CI-Job rot/grün pro Testfall, Report als Markdown-Artefakt.
