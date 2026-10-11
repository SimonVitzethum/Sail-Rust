//! sail-instrs-core — Kern-Shard-Analyse für Lane 33.
//!
//! Streaming-Analyse (O(1) Zusatzspeicher relativ zur Dateigröße) der drei
//! ARMv9-Kern-Dateien `instrs32.sail`, `instrs64.sail` und `v8_base.sail`:
//!
//! - [`scanner`]: erkennt Top-Level-Definitionen zeilenweise per [`BufRead`].
//! - [`plan`]: ordnet jede Definition einem Kern-Shard zu (execute / decode /
//!   dispatch / helper).
//! - [`dispatch`]: extrahiert Decode→Execute-Kanten und Clause→Decode-Kanten
//!   inkl. `SEE`-Prioritätswächter.
//! - [`measure`]: vermisst Funktionsrümpfe (Zeilen, Match-Arme `=>`, `if`s).
//!
//! Kein Full-Build: Diese Crate parst niemals das Gesamtmodell, sondern immer
//! nur eine Datei als Strom. Details und Messwerte stehen in `WORK_STATE.md`.
#![forbid(unsafe_code)]

pub mod dispatch;
pub mod measure;
pub mod plan;
pub mod scanner;
