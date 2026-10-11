//! Bounded Parse → Check → Emit orchestration using one shared worker pool.
//! Lazy inputs, completed results and reorder buffers share the scheduler's
//! credit window. Collecting entry points are compatibility adapters; only
//! `run_pipeline_streaming` permits corpus-independent memory consumption.

use sail_shard::Shard;
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

/// Dreistufige Pipeline über Shards (Implementierung, Lane 84).
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
    run_shard_parallel(shards, parse, check, emit, config)
}

/// Ein-poolige Variante: ganze Kette Parse→Check→Emit pro Shard in genau
/// einem Worker auf dem speicherbegrenzten Streaming-Scheduler.
/// Die sammelnde Rückgabe hält alle Ergebnisse; für große Modelle
/// `run_pipeline_streaming` mit einem Dateischreiber verwenden.
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
    let mut out = Vec::with_capacity(shards.len());
    run_pipeline_streaming(shards, parse, check, emit, config, |item| {
        out.push(item);
        Ok::<_, std::convert::Infallible>(())
    }).expect("infallible collector");
    out
}

/// Memory-bounded Parse → Check → Emit over lazy inputs, with ordered output.
///
/// One worker runs the complete chain for each shard. This shares a single
/// CPU budget and releases intermediate ASTs immediately, avoiding three
/// competing pools and queued AST copies. At most `max_inflight +
/// chain_workers()` shards (including completed results) are resident.
/// `consume` should spill each output to disk rather than retain its body.
/// Its first error stops ingress; stage errors remain per-shard results.
/// Unlike collecting variants, total corpus size does not determine RAM use.
pub fn run_pipeline_streaming<I, P, C, E, FP, FC, FE, S, Error>(
    shards: I,
    parse: FP,
    check: FC,
    emit: FE,
    config: PipeConfig,
    mut consume: S,
) -> Result<(), Error>
where
    I: IntoIterator<Item = Shard>,
    I::IntoIter: Send,
    P: Send,
    C: Send,
    E: Send,
    FP: Fn(Shard) -> Result<P, String> + Sync,
    FC: Fn(P) -> Result<C, String> + Sync,
    FE: Fn(C) -> Result<E, String> + Sync,
    S: FnMut(ShardOutput<E>) -> Result<(), Error>,
{
    use sail_shard::{Scheduler, SchedulerConfig};
    let sched = Scheduler::new(SchedulerConfig::new(
        config.max_inflight, config.chain_workers(),
    ));
    sched.run_streaming(shards, |shard| {
        let id = shard.id.to_string();
        let output = parse(shard)
            .map_err(|message| StageError::new(Stage::Parse, id.clone(), message))
            .and_then(|p| check(p)
                .map_err(|message| StageError::new(Stage::Check, id.clone(), message)))
            .and_then(|c| emit(c)
                .map_err(|message| StageError::new(Stage::Emit, id.clone(), message)));
        (id, output)
    }, |order, (shard, output)| consume(ShardOutput { order, shard, output }))
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

    #[test]
    fn default_backpressure_is_8() {
        assert_eq!(PipeConfig::default().max_inflight, 8);
        assert_eq!(PipeConfig::default_bounded().max_inflight, 8);
    }

    #[test]
    fn default_workers_follow_cores() {
        let expect = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let cfg = PipeConfig::default_bounded();
        assert_eq!(cfg.parse_workers, expect);
        assert_eq!(cfg.check_workers, expect);
        assert_eq!(cfg.emit_workers, expect);
    }

    #[test]
    fn pipelined_order_stable_under_jitter() {
        // Spaete Shards absichtlich schneller: Merge-Ordnung muss trotzdem
        // der Eingangsreihenfolge entsprechen.
        let cfg = PipeConfig::new(8, 4, 4, 4);
        let out = run_pipeline(
            shards(16),
            |s: Shard| {
                let n = s.contents.len() % 8;
                std::thread::sleep(std::time::Duration::from_millis((8 - n) as u64));
                Ok::<_, String>(s.contents)
            },
            |c: String| Ok::<_, String>(c),
            |c: String| Ok::<_, String>(c),
            cfg,
        );
        assert_eq!(out.len(), 16);
        for (i, o) in out.iter().enumerate() {
            assert_eq!(o.order, i);
            assert_eq!(o.shard, format!("s{i}"));
            assert!(o.output.is_ok(), "shard {i} lost");
        }
    }

    #[test]
    fn pipelined_serial_backpressure_stays_ordered() {
        // Engste Backpressure (max_inflight = 1, je ein Worker): kein
        // Deadlock, Ordnung erhalten.
        let cfg = PipeConfig::new(1, 1, 1, 1);
        let out = run_pipeline(
            shards(10),
            |s: Shard| Ok::<_, String>(s.contents),
            |c: String| Ok::<_, String>(c),
            |c: String| Ok::<_, String>(c),
            cfg,
        );
        assert_eq!(out.len(), 10);
        for (i, o) in out.iter().enumerate() {
            assert_eq!(o.order, i);
            assert!(o.output.is_ok(), "shard {i} lost");
        }
    }

    #[test]
    fn pipelined_check_emit_errors_isolated() {
        let cfg = PipeConfig::new(4, 2, 2, 2);
        let out = run_pipeline(
            shards(4),
            |s: Shard| Ok::<_, String>(s.contents),
            |c: String| {
                if c.contains("f1") {
                    Err("check-fehler".to_string())
                } else {
                    Ok(c)
                }
            },
            |c: String| {
                if c.contains("f3") {
                    Err("emit-fehler".to_string())
                } else {
                    Ok(c)
                }
            },
            cfg,
        );
        assert_eq!(out.len(), 4);
        let e1 = out[1].output.as_ref().err().expect("s1 fails in check");
        assert_eq!(e1.stage, Stage::Check);
        assert_eq!(e1.shard, "s1");
        let e3 = out[3].output.as_ref().err().expect("s3 fails in emit");
        assert_eq!(e3.stage, Stage::Emit);
        assert_eq!(e3.shard, "s3");
        assert!(out[0].output.is_ok());
        assert!(out[2].output.is_ok());
    }

    #[test]
    fn chain_variant_error_isolated() {
        let cfg = PipeConfig::new(4, 2, 2, 2);
        let out: Vec<ShardOutput<String>> = run_shard_parallel(
            shards(3),
            |s: Shard| {
                if s.id.name.as_str() == "s0" {
                    Err("parse-fehler".to_string())
                } else {
                    Ok(s.contents)
                }
            },
            |c: String| Ok::<_, String>(c),
            |c: String| Ok::<_, String>(c),
            cfg,
        );
        assert_eq!(out.len(), 3);
        let err = out[0].output.as_ref().err().expect("s0 fails");
        assert_eq!(err.stage, Stage::Parse);
        assert_eq!(err.shard, "s0");
        assert!(out[1].output.is_ok());
        assert!(out[2].output.is_ok());
    }
}
