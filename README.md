# sail-rs — Sail-Compiler in reinem Rust
Ziel: Sail→Lean in purem Rust, `#![forbid(unsafe_code)]`, low-RAM + multithreaded.
Jede Lane arbeitet in eigenem Worktree auf eigenem Branch `lane/<name>`.
Nach Merge: Worktree löschen (`git worktree remove`), Branch löschen.
