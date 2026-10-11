# Gesicherte Prüfnachweise — 2026-10-11

Diese unveränderten Logs wurden aus den Arbeitskopien unter `/tmp` ins
Repository kopiert. Der aktuelle Gesamtstand steht in
[WORK_STATE.md](../../WORK_STATE.md). Historische Erfolge gelten nicht automatisch
für den neuesten Worktree.

| Log | Einordnung |
| --- | --- |
| [mutable-focused-128M.log](mutable-focused-128M.log) | Neuester Lauf: **fehlgeschlagen**, kanonische Suite OOM bei 128 MiB. 25 andere Programme bestehen mit zusammen 333 Tests. |
| [mutable-before-deep-test-128M.log](mutable-before-deep-test-128M.log) | Früherer Mutable-Zwischenstand: 36 kanonische Tests bestanden; noch ohne neuen 10.000-Level-Mutable-/Loop-Test. |
| [mutable-ocaml-positive-256M.log](mutable-ocaml-positive-256M.log) | 13 positive Mutable-/Loop-Referenzfälle angenommen. |
| [mutable-ocaml-negative-256M.log](mutable-ocaml-negative-256M.log) | 19 negative Referenzfälle jeweils abgelehnt. Aktuelle Fixture-Erweiterungen sind darin nicht enthalten. |
| [arm-vectors-16G.log](arm-vectors-16G.log) | Vollständiger ARM-Eingang, Abbruch bei `to_bytes_le`; kein erfolgreicher Typecheck/Lean-Build. Vor aktuellen Mutable-Änderungen. |
| [dependent-calls-ooc-128M.log](dependent-calls-ooc-128M.log) | 16-Worker-Teilmengenprüfung mit größerem AST als Speicherlimit; erwartetes CLI-Ende wegen fehlendem Backend. |
| [vector-focused-128M.log](vector-focused-128M.log) | Frühere Vektor-Fokussuite: 322 Tests bestanden. |
| [vector-canonical-128M.log](vector-canonical-128M.log) | Frühere Vektor-Kanoniksuite: 35 Tests bestanden. |

Systemd-Endstatus und cgroup-Peak im jeweiligen Log sind maßgeblich. Eine Liste
bestandener Einzeltests ohne erfolgreichen Programmabschluss ist kein grüner
Suite-Nachweis. CLI-Profilwerte und cgroup-Peaks messen unterschiedliche Dinge;
SMT-Zähler sind für den kanonischen Prover noch nicht vollständig verdrahtet.
