//! Dreistufige Shard-Pipeline Parse → Check → Emit (Skizze).
//!
//! Design-Regeln (von Lane 11/43 übernommen):
//! * Stufen-Weiterreichung über **bounded** `crossbeam-channel`-Kanäle mit
//!   Kapazität `max_inflight` — blockierendes `send` = Backpressure.
//! * Ergebniskanal **unbegrenzt** (Deadlock-Freiheit, vgl.
//!   [`sail_shard::Scheduler`]-Doku): begrenzt + erst-nach-Produktion-lesen
//!   verklemmt bei mehreren Workern.
//! * Produzent, alle Worker-Pools und Konsument laufen nebenläufig in einem
//!   `thread::scope`; Kanal-Schließung kaskadiert (Produzent fertig → Parse
//!   drainiert → Check drainiert → Emit drainiert).
//! * Jede Eingabe trägt ihren Eingangsindex; der Konsument sortiert vor der
//!   Rückgabe nach diesem Index (deterministischer Merge).
//! * Panics sind fail-fast (Scope propagiert); erwartete Stufen-Fehler sind
//!   pro-Shard-`Result`s ([`StageError`]) und verlieren keine Nachbar-Shards.

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use sail_shard::Shard;
use std::sync::Arc;
use std::thread;

/// Pipeline-Stufe, für die Fehlerattribution in [`StageError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Shard-Text → Stufen-Eingabe (z. B. `sail-parse`).
    Parse,
    /// Konsistenz-/Typprüfung (z. B. `sail-ty`).
    Check,
    /// Zielcode-Erzeugung (z. B. `sail-lean`).
    Emit,
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse => write!(f, "parse"),
            Self::Check => write!(f, "check"),
            Self::Emit => write!(f, "emit"),
        }
    }
}

/// Fehler genau eines Shards in genau einer Stufe.
///
/// Downstream-Stufen reichen ihn unverändert durch, ohne ihre Closure
/// aufzurufen; der Shard belegt danach nur noch seinen Ergebnis-Slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageError {
    /// Stufe, in der der Fehler entstand.
    pub stage: Stage,
    /// `ShardId`-Anzeige (`name` bzw. `name#chunk`).
    pub shard: String,
    /// Stufen-Fehlermeldung (Closure-`Err`-Text).
    pub message: String,
}

impl StageError {
    /// Erzeugt einen Stufen-Fehler.
    #[must_use]
    pub fn new(stage: Stage, shard: String, message: String) -> Self {
        Self {
            stage,
            shard,
            message,
        }
    }
}

impl std::fmt::Display for StageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.stage, self.shard, self.message)
    }
}

impl std::error::Error for StageError {}

/// Konfiguration der Pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipeConfig {
    /// Max. wartende Shards pro Stufen-Queue (= Kanal-Kapazität).
    /// Low-RAM-Schranke, Lane-11-Konvention: Default 8.
    pub max_inflight: usize,
    /// Worker-Threads der Parse-Stufe.
    pub parse_workers: usize,
    /// Worker-Threads der Check-Stufe.
    pub check_workers: usize,
    /// Worker-Threads der Emit-Stufe.
    pub emit_workers: usize,
}

impl PipeConfig {
    /// Neue Konfiguration; Werte `< 1` werden auf `1` normalisiert.
    #[must_use]
    pub fn new(
        max_inflight: usize,
        parse_workers: usize,
        check_workers: usize,
        emit_workers: usize,
    ) -> Self {
        Self {
            max_inflight: max_inflight.max(1),
            parse_workers: parse_workers.max(1),
            check_workers: check_workers.max(1),
            emit_workers: emit_workers.max(1),
        }
    }

    /// Sinnvoller Default: 8 inflight (Lane-Spec), jede Stufe = Kerne.
    /// Threads ohne Arbeit blockieren billig auf leeren Kanälen; die
    /// Speicher-Schranke kommt aus `max_inflight`, nicht aus der Thread-Zahl.
    #[must_use]
    pub fn default_bounded() -> Self {
        let workers = thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        Self::new(8, workers, workers, workers)
    }

    /// Worker-Zahl für die ein-poolige Variante
    /// ([`run_shard_parallel`]): Maximum der Stufen-Pools.
    #[must_use]
    pub fn chain_workers(&self) -> usize {
        self.parse_workers
            .max(self.check_workers)
            .max(self.emit_workers)
    }
}

impl Default for PipeConfig {
    fn default() -> Self {
        Self::default_bounded()
    }
}

/// Ausgabe eines Shards nach deterministischem Merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardOutput<E> {
    /// Eingangsposition des Shards (Merge-Schlüssel).
    pub order: usize,
    /// `ShardId`-Anzeige (`name` bzw. `name#chunk`).
    pub shard: String,
    /// Stufen-Ergebnis (`Err` = [`StageError`] der zuerst fehlschlagenden Stufe).
    pub output: Result<E, StageError>,
}

/// Dreistufige Pipeline über Shards (Skizze).
///
/// * `shards`: Eingaben in **kanonischer Ordnung** (z. B. Lane-11
///   `shard_files`-Reihenfolge); die Rückgabe steht in derselben Ordnung.
/// * `parse`/`check`/`emit`: reine Stufen-Funktionen (`Send + Sync`, von allen
///   Workern der Stufe geteilt). `Err(String)` bricht nur den betroffenen
///   Shard ab.
/// * Rückgabe: ein [`ShardOutput`] pro Eingabe-Shard, sortiert nach `order`.
///
/// Noch kein Treiber-Umbau: echte `sail-parse`-/`sail-ty`-/`sail-lean`-Aufrufe
/// werden später als Closures injiziert.
pub fn run_pipeline<P, C, E, FP, FC, FE>(
    shards: Vec<Shard>,
    parse: FP,
    check: FC,
    emit: FE,
    config: PipeConfig,
) -> Vec<ShardOutput<E>>
where
    P: Send + 'static,
    C: Send + 'static,
    E: Send + 'static,
    FP: Fn(Shard) -> Result<P, String> + Send + Sync + 'static,
    FC: Fn(P) -> Result<C, String> + Send + Sync + 'static,
    FE: Fn(C) -> Result<E, String> + Send + Sync + 'static,
{
    let n = shards.len();
    if n == 0 {
        return Vec::new();
    }
    let cap = config.max_inflight;
    let np = config.parse_workers.min(n).max(1);
    let nc = config.check_workers.min(n).max(1);
    let ne = config.emit_workers.min(n).max(1);

    // Stufen-Kanäle bounded (Backpressure), Ergebniskanal unbegrenzt
    // (Deadlock-Freiheit, Lane-11-Begründung).
    let (parse_tx, parse_rx): (Sender<(usize, Shard)>, Receiver<(usize, Shard)>) =
        bounded(cap);
    let (check_tx, check_rx): (
        Sender<(usize, String, Result<P, StageError>)>,
        Receiver<(usize, String, Result<P, StageError>)>,
    ) = bounded(cap);
    let (emit_tx, emit_rx): (
        Sender<(usize, String, Result<C, StageError>)>,
        Receiver<(usize, String, Result<C, StageError>)>,
    ) = bounded(cap);
    let (res_tx, res_rx): (
        Sender<(usize, String, Result<E, StageError>)>,
        Receiver<(usize, String, Result<E, StageError>)>,
    ) = unbounded();

    let parse = Arc::new(parse);
    let check = Arc::new(check);
    let emit = Arc::new(emit);

    thread::scope(|s| {
        // Parse-Pool.
        for _ in 0..np {
            let rx = parse_rx.clone();
            let tx = check_tx.clone();
            let parse = Arc::clone(&parse);
            s.spawn(move || {
                for (order, shard) in rx {
                    let id = shard.id.to_string();
                    let out = parse(shard)
                        .map_err(|message| StageError::new(Stage::Parse, id.clone(), message));
                    // Send-Fehler nur, wenn der Empfänger weg ist (Panic-Pfad).
                    let _ = tx.send((order, id, out));
                }
            });
        }
        drop(parse_rx);
        drop(check_tx);

        // Check-Pool: `Err` wird durchgereicht, Closure nicht aufgerufen.
        for _ in 0..nc {
            let rx = check_rx.clone();
            let tx = emit_tx.clone();
            let check = Arc::clone(&check);
            s.spawn(move || {
                for (order, id, parsed) in rx {
                    let out = match parsed {
                        Err(e) => Err(e),
                        Ok(p) => check(p).map_err(|message| {
                            StageError::new(Stage::Check, id.clone(), message)
                        }),
                    };
                    let _ = tx.send((order, id, out));
                }
            });
        }
        drop(check_rx);
        drop(emit_tx);

        // Emit-Pool.
        for _ in 0..ne {
            let rx = emit_rx.clone();
            let tx = res_tx.clone();
            let emit = Arc::clone(&emit);
            s.spawn(move || {
                for (order, id, checked) in rx {
                    let out = match checked {
                        Err(e) => Err(e),
                        Ok(c) => emit(c).map_err(|message| {
                            StageError::new(Stage::Emit, id.clone(), message)
                        }),
                    };
                    let _ = tx.send((order, id, out));
                }
            });
        }
        drop(emit_rx);
        drop(res_tx);

        // Produzent: blockiert bei voller Parse-Queue (bounded, low-RAM).
        // Danach schließt der Kanal kaskadierend alle Pools.
        s.spawn(move || {
            for (order, shard) in shards.into_iter().enumerate() {
                parse_tx.send((order, shard)).expect("parse worker alive");
            }
        });

        // Konsument: exakt n Ergebnisse, dann deterministischer Merge.
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let (order, id, result) = res_rx.recv().expect("emit worker alive");
            out.push(ShardOutput {
                order,
                shard: id,
                output: result,
            });
        }
        out.sort_by_key(|o| o.order);
        out
    })
}

/// Ein-poolige Variante: ganze Kette Parse→Check→Emit pro Shard in genau
/// einem Worker, direkt auf [`sail_shard::Scheduler::run_with_index`]
/// (Lane 11) aufsetzend. Kein Stufen-Überlapp, aber minimale
/// Synchronisation und dieselbe Merge-Garantie (nach `order` sortiert).
/// Worker-Zahl = [`PipeConfig::chain_workers`].
pub fn run_shard_parallel<P, C, E, FP, FC, FE>(
    shards: Vec<Shard>,
    parse: FP,
    check: FC,
    emit: FE,
    config: PipeConfig,
) -> Vec<ShardOutput<E>>
where
    P: Send + 'static,
    C: Send + 'static,
    E: Send + 'static,
    FP: Fn(Shard) -> Result<P, String> + Send + Sync + 'static,
    FC: Fn(P) -> Result<C, String> + Send + Sync + 'static,
    FE: Fn(C) -> Result<E, String> + Send + Sync + 'static,
{
    use sail_shard::{Scheduler, SchedulerConfig};

    if shards.is_empty() {
        return Vec::new();
    }
    let workers = config.chain_workers().min(shards.len()).max(1);
    let sched = Scheduler::new(SchedulerConfig::new(config.max_inflight, workers));
    let parse = Arc::new(parse);
    let check = Arc::new(check);
    let emit = Arc::new(emit);

    let indexed = sched.run_with_index(shards, move |shard: Shard| {
        let id = shard.id.to_string();
        let parsed = match parse(shard) {
            Ok(v) => v,
            Err(message) => return (id.clone(), Err((Stage::Parse, message))),
        };
        let checked = match check(parsed) {
            Ok(v) => v,
            Err(message) => return (id.clone(), Err((Stage::Check, message))),
        };
        match emit(checked) {
            Ok(v) => (id, Ok(v)),
            Err(message) => (id.clone(), Err((Stage::Emit, message))),
        }
    });
    let mut out: Vec<ShardOutput<E>> = indexed
        .into_iter()
        .map(|(order, (id, r))| {
            let output =
                r.map_err(|(stage, message)| StageError::new(stage, id.clone(), message));
            ShardOutput {
                order,
                shard: id,
                output,
            }
        })
        .collect();
    out.sort_by_key(|o| o.order);
    out
}

// Tests als Code (E3: NICHT ausführen; Verifikation via Test-Rig, später).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::manifest;
    use sail_shard::{ShardId, ShardKind};

    fn shard(name: &str, chunk: u32, contents: &str) -> Shard {
        Shard {
            id: ShardId::new(name, chunk),
            kind: ShardKind::File,
            start_line: 1,
            end_line: contents.lines().count() + 1,
            contents: contents.to_string(),
        }
    }

    fn shards(n: usize) -> Vec<Shard> {
        (0..n)
            .map(|i| shard(&format!("s{i}"), 0, &format!("def f{i} := {i}")))
            .collect()
    }

    #[test]
    fn config_normalizes_zeros() {
        let c = PipeConfig::new(0, 0, 0, 0);
        assert_eq!(c.max_inflight, 1);
        assert_eq!(c.parse_workers, 1);
        assert_eq!(c.check_workers, 1);
        assert_eq!(c.emit_workers, 1);
    }

    #[test]
    fn pipelined_preserves_input_order() {
        let cfg = PipeConfig::new(2, 4, 4, 4);
        let out = run_pipeline(
            shards(30),
            |s: Shard| Ok::<_, String>(s.contents.len()),
            |len: usize| Ok::<_, String>(len + 1),
            |c: usize| Ok::<_, String>(c.to_string()),
            cfg,
        );
        assert_eq!(out.len(), 30);
        for (i, o) in out.iter().enumerate() {
            assert_eq!(o.order, i);
            assert_eq!(o.shard, format!("s{i}"));
            assert!(o.output.is_ok(), "shard {i} lost");
        }
    }

    #[test]
    fn pipelined_error_isolated_to_shard() {
        let cfg = PipeConfig::new(4, 2, 2, 2);
        let out = run_pipeline(
            shards(5),
            |s: Shard| {
                if s.id.name.as_str() == "s2" {
                    Err("kaputt".to_string())
                } else {
                    Ok(s.contents)
                }
            },
            |c: String| Ok::<_, String>(c),
            |c: String| Ok::<_, String>(c),
            cfg,
        );
        assert_eq!(out.len(), 5);
        for (i, o) in out.iter().enumerate() {
            assert_eq!(o.order, i);
        }
        let err = out[2].output.as_ref().err().expect("s2 fails");
        assert_eq!(err.stage, Stage::Parse);
        assert_eq!(err.shard, "s2");
        for (i, o) in out.iter().enumerate() {
            if i != 2 {
                assert!(o.output.is_ok(), "shard {i} lost");
            }
        }
    }

    #[test]
    fn pipelined_empty_input_returns_empty() {
        let out = run_pipeline(
            Vec::new(),
            |s: Shard| Ok::<_, String>(s),
            |p: Shard| Ok::<_, String>(p),
            |c: Shard| Ok::<_, String>(c),
            PipeConfig::default(),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn chain_variant_matches_pipelined_manifest() {
        let cfg = PipeConfig::new(8, 4, 4, 4);
        // Beide Läufe bekommen frische Shards; Stufen als fn-Items
        // (eigene Instanzen pro Aufruf, da Closures nicht `Clone` sind).
        fn p(s: Shard) -> Result<String, String> {
            Ok(s.contents)
        }
        fn c(s: String) -> Result<usize, String> {
            Ok(s.len())
        }
        fn e(n: usize) -> Result<String, String> {
            Ok(n.to_string())
        }
        let piped = run_pipeline(shards(20), p, c, e, cfg);
        let chained = run_shard_parallel(shards(20), p, c, e, cfg);
        assert_eq!(manifest(&piped), manifest(&chained));
        assert_eq!(piped.len(), chained.len());
    }

    #[test]
    fn chain_variant_empty_input_returns_empty() {
        let out: Vec<ShardOutput<String>> = run_shard_parallel(
            Vec::new(),
            |s: Shard| Ok::<_, String>(s.contents),
            |c: String| Ok::<_, String>(c),
            |c: String| Ok::<_, String>(c),
            PipeConfig::default(),
        );
        assert!(out.is_empty());
    }
}
