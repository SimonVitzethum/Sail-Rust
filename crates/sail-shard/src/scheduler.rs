//! Parallel-Scheduler mit begrenzter Queue (low-RAM, multithreaded).
//!
//! Design: `crossbeam-channel`-Queue mit Kapazitaet `max_inflight` fuer
//! anstehende Arbeit + Worker-Threads (`num_workers`). Der Produzent blockiert,
//! sobald mehr als `max_inflight` Shards unterwegs sind — der Speicher fuer
//! *wartende Eingaben* bleibt beschraenkt, egal wie viele Shards es gibt.
//! Ergebnisse laufen ueber einen unbegrenzten Kanal: Sie muessen ohnehin alle
//! im Rueckgabe-`Vec` gehalten werden, und ein begrenzter Ergebniskanal koennte
//! bei mehreren Workern verklemmen (Produzent blockiert auf voller Arbeits-,
//! Worker auf voller Ergebnis-Queue, Konsument liest erst nach dem Produzieren).
//! Zusaetzlich gibt es eine `rayon`-Variante fuer rein datenparallele,
//! nicht-blockierende Maps.

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use std::thread;

/// Kanal-Nutzdaten: `(Input-Index, Wert)` — Index für
/// [`Scheduler::run_with_index`]/[`Scheduler::run_ordered`].
type WorkItem<T> = (usize, T);
/// Kanal-Nutzdaten der Ergebnisse: `(Input-Index, Output)`.
type WorkResult<R> = (usize, R);

/// Konfiguration des Schedulers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerConfig {
    /// Max. gleichzeitig inflight Shards (= Queue-Kapazitaet). Low-RAM-Schranke.
    pub max_inflight: usize,
    /// Anzahl Worker-Threads.
    pub num_workers: usize,
}

impl SchedulerConfig {
    /// Neue Konfiguration; Werte `< 1` werden auf `1` normalisiert.
    #[must_use]
    pub fn new(max_inflight: usize, num_workers: usize) -> Self {
        Self {
            max_inflight: max_inflight.max(1),
            num_workers: num_workers.max(1),
        }
    }

    /// Sinnvoller Default: 8 inflight (Lane-Spec), Worker = verfügbare Kerne.
    #[must_use]
    pub fn default_bounded() -> Self {
        let workers = thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        Self::new(8, workers)
    }
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self::default_bounded()
    }
}

/// Parallel-Scheduler ueber Shards.
#[derive(Debug, Clone, Copy)]
pub struct Scheduler {
    config: SchedulerConfig,
}

impl Scheduler {
    /// Erzeugt einen Scheduler mit der gegebenen Konfiguration.
    #[must_use]
    pub fn new(config: SchedulerConfig) -> Self {
        Self { config }
    }

    /// Aktuelle Konfiguration.
    #[must_use]
    pub fn config(&self) -> SchedulerConfig {
        self.config
    }

    /// Fuehrt `f` fuer jedes Item in `items` parallel aus.
    ///
    /// * Begrenzung: hoechstens `max_inflight` *wartende* Items sind
    ///   gleichzeitig eingereiht (bounded Arbeits-Channel); dazu kommen die
    ///   gerade in `num_workers` Threads bearbeiteten Items. Der Ergebnis-
    ///   Channel ist bewusst unbegrenzt (Deadlock-Freiheit, siehe Modul-Doku).
    /// * `f` muss `Send + Sync` sein und wird von `num_workers` Threads
    ///   geteilt (Funktionszeiger oder `&dyn`-Closure-Wrapper).
    /// * Ergebnisse kommen in Abschlussreihenfolge zurueck; mit
    ///   [`Scheduler::run_ordered`] indexiert und sortiert.
    pub fn run<T, R, F>(&self, items: Vec<T>, f: F) -> Vec<R>
    where
        T: Send + 'static,
        R: Send + 'static,
        F: Fn(T) -> R + Send + Sync + 'static,
    {
        let unordered = self.run_with_index(items, f);
        unordered.into_iter().map(|(_, r)| r).collect()
    }

    /// Wie [`Scheduler::run`], gibt aber `(input_index, output)`-Paare zurueck.
    pub fn run_with_index<T, R, F>(&self, items: Vec<T>, f: F) -> Vec<(usize, R)>
    where
        T: Send + 'static,
        R: Send + 'static,
        F: Fn(T) -> R + Send + Sync + 'static,
    {
        use std::sync::Arc;
        let n = items.len();
        if n == 0 {
            return Vec::new();
        }
        let workers = self.config.num_workers.min(n).max(1);
        let cap = self.config.max_inflight;

        let (work_tx, work_rx): (Sender<WorkItem<T>>, Receiver<WorkItem<T>>) = bounded(cap);
        // Ergebniskanal unbegrenzt: begrenzt + erst-nach-Produktion-lesen
        // verklemmt bei mehreren Workern (siehe Modul-Doku).
        let (res_tx, res_rx): (Sender<WorkResult<R>>, Receiver<WorkResult<R>>) = unbounded();

        let f = Arc::new(f);
        thread::scope(|s| {
            // Worker: ziehen Jobs, bis der Kanal geschlossen ist.
            for worker in 0..workers {
                let rx = work_rx.clone();
                let tx = res_tx.clone();
                let f = Arc::clone(&f);
                thread::Builder::new().name(format!("sail-worker-{worker}")).spawn_scoped(s, move || {
                    for (i, item) in rx {
                        let out = f(item);
                        // Send-Fehler nur, wenn der Empfaenger weg ist (Panic-Pfad).
                        let _ = tx.send((i, out));
                    }
                }).expect("cannot spawn scheduler worker");
            }
            drop(res_tx);
            drop(work_rx);

            // Produzent im eigenen Thread: blockiert bei voller Arbeits-Queue
            // (bounded, low-RAM), waehrend der Scope-Thread gleichzeitig
            // Ergebnisse konsumiert. Produzieren und Konsumieren laufen also
            // nebenlaeufig; zusammen mit dem unbegrenzten Ergebniskanal kann
            // nichts verklemmen.
            thread::Builder::new().name("sail-ingress".into()).spawn_scoped(s, move || {
                for (i, item) in items.into_iter().enumerate() {
                    work_tx.send((i, item)).expect("worker alive");
                }
            }).expect("cannot spawn scheduler ingress");

            // Konsument: exakt n Ergebnisse einsammeln.
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                let r = res_rx.recv().expect("worker alive");
                out.push(r);
            }
            out
        })
    }

    /// Wie [`Scheduler::run`], Ergebnisse in Input-Reihenfolge sortiert.
    pub fn run_ordered<T, R, F>(&self, items: Vec<T>, f: F) -> Vec<R>
    where
        T: Send + 'static,
        R: Send + 'static,
        F: Fn(T) -> R + Send + Sync + 'static,
    {
        let mut indexed = self.run_with_index(items, f);
        indexed.sort_by_key(|(i, _)| *i);
        indexed.into_iter().map(|(_, r)| r).collect()
    }

    /// Process a lazy input stream in parallel and consume results in input order.
    ///
    /// Unlike the collecting APIs, neither inputs nor outputs accumulate with
    /// the size of the corpus. A credit window bounds *all* outstanding items
    /// (queued, running, completed and waiting for an earlier result) to
    /// `max_inflight + num_workers`. Credits are returned only after `consume`
    /// finishes, so a slow first item or a slow disk writer applies backpressure
    /// all the way to the input iterator. Each item must itself fit in memory.
    ///
    /// `consume` can write results to disk and release them immediately. Its
    /// first error cancels ingress; running calls finish before returning.
    /// Worker panics also cancel ingress and are rethrown after threads join.
    pub fn run_streaming<I, T, R, F, S, E>(
        &self,
        items: I,
        f: F,
        mut consume: S,
    ) -> Result<(), E>
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: Send,
        T: Send,
        R: Send,
        F: Fn(T) -> R + Sync,
        S: FnMut(usize, R) -> Result<(), E>,
    {
        use std::collections::BTreeMap;
        use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

        let items = items.into_iter();
        let workers = self.config.num_workers.max(1);
        let cap = self.config.max_inflight.max(1);
        let window = cap.saturating_add(workers);
        let outcome = thread::scope(|scope| {
            let (work_tx, work_rx) = bounded::<WorkItem<T>>(cap);
            let (res_tx, res_rx) = bounded(window);
            let (credit_tx, credit_rx) = bounded(window);
            for _ in 0..window {
                credit_tx.send(()).expect("credit receiver alive");
            }
            let f = &f;
            for worker in 0..workers {
                let rx = work_rx.clone();
                let tx = res_tx.clone();
                thread::Builder::new().name(format!("sail-worker-{worker}")).spawn_scoped(scope, move || {
                    for (order, item) in rx {
                        let result = catch_unwind(AssertUnwindSafe(|| f(item)));
                        let failed = result.is_err();
                        if tx.send((order, result)).is_err() || failed {
                            break;
                        }
                    }
                }).expect("cannot spawn scheduler worker");
            }
            drop(work_rx);
            drop(res_tx);
            thread::Builder::new().name("sail-ingress".into()).spawn_scoped(scope, move || {
                let mut items = items.enumerate();
                // Acquire BEFORE next(): lazy loading must also fit the window.
                while credit_rx.recv().is_ok() {
                    let Some(item) = items.next() else { break };
                    if work_tx.send(item).is_err() {
                        break;
                    }
                }
            }).expect("cannot spawn scheduler ingress");

            let mut next = 0;
            let mut pending = BTreeMap::new();
            let mut outcome = Ok(Ok(()));
            'receive: for (order, result) in &res_rx {
                let value = match result {
                    Ok(value) => value,
                    Err(panic) => {
                        outcome = Err(panic);
                        break;
                    }
                };
                pending.insert(order, value);
                while let Some(value) = pending.remove(&next) {
                    if let Err(err) = consume(next, value) {
                        outcome = Ok(Err(err));
                        break 'receive;
                    }
                    next += 1;
                    // Ingress may already have reached EOF.
                    let _ = credit_tx.send(());
                }
            }
            // Unblock ingress and workers before scope joins them, including
            // failure paths with a full work or result queue.
            drop(credit_tx);
            drop(res_rx);
            outcome
        });
        match outcome {
            Ok(result) => result,
            Err(panic) => resume_unwind(panic),
        }
    }

    /// Rayon-Variante: datenparalleles Map ohne Queue (fuer kleine,
    /// speicherarme Closures); nutzt den globalen rayon-Pool.
    pub fn run_rayon<T, R, F>(items: Vec<T>, f: F) -> Vec<R>
    where
        T: Send,
        R: Send,
        F: Fn(T) -> R + Send + Sync,
    {
        use rayon::prelude::*;
        items.into_par_iter().map(f).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn config_normalizes_zeros() {
        let c = SchedulerConfig::new(0, 0);
        assert_eq!(c.max_inflight, 1);
        assert_eq!(c.num_workers, 1);
    }

    #[test]
    fn run_completes_all_items() {
        let sched = Scheduler::new(SchedulerConfig::new(8, 4));
        let out = sched.run((0..100).collect::<Vec<_>>(), |x: i32| x * 2);
        let mut sorted = out;
        sorted.sort_unstable();
        assert_eq!(sorted, (0..100).map(|x| x * 2).collect::<Vec<_>>());
    }

    #[test]
    fn run_ordered_preserves_input_order() {
        let sched = Scheduler::new(SchedulerConfig::new(8, 4));
        let out = sched.run_ordered((0..50).collect::<Vec<_>>(), |x: i32| x + 1);
        assert_eq!(out, (1..51).collect::<Vec<_>>());
    }

    #[test]
    fn max_inflight_is_respected() {
        // Misst die max. gleichzeitige Nebenlaeufigkeit; sie darf die Zahl
        // der Worker nicht uebersteigen und bei max_inflight=1/worker=1
        // strikt seriell sein.
        let sched = Scheduler::new(SchedulerConfig::new(1, 1));
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let l2 = Arc::clone(&live);
        let p2 = Arc::clone(&peak);
        let n = 20;
        let out = sched.run((0..n).collect::<Vec<_>>(), move |x: i32| {
            let cur = l2.fetch_add(1, Ordering::SeqCst) + 1;
            p2.fetch_max(cur, Ordering::SeqCst);
            thread::sleep(std::time::Duration::from_millis(1));
            l2.fetch_sub(1, Ordering::SeqCst);
            x
        });
        assert_eq!(out.len(), n as usize);
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn bounded_queue_limits_peak_below_items() {
        let sched = Scheduler::new(SchedulerConfig::new(2, 2));
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let l2 = Arc::clone(&live);
        let p2 = Arc::clone(&peak);
        let out = sched.run((0..50).collect::<Vec<_>>(), move |x: i32| {
            let cur = l2.fetch_add(1, Ordering::SeqCst) + 1;
            p2.fetch_max(cur, Ordering::SeqCst);
            thread::yield_now();
            l2.fetch_sub(1, Ordering::SeqCst);
            x * x
        });
        assert_eq!(out.len(), 50);
        assert!(peak.load(Ordering::SeqCst) <= 2, "peak unbounded!");
    }

    #[test]
    fn run_rayon_matches_serial() {
        let out = Scheduler::run_rayon(vec![1, 2, 3, 4], |x: i32| x * 10);
        let mut s = out;
        s.sort_unstable();
        assert_eq!(s, vec![10, 20, 30, 40]);
    }

    #[test]
    fn empty_input_returns_empty() {
        let sched = Scheduler::new(SchedulerConfig::default());
        let out: Vec<i32> = sched.run(Vec::<i32>::new(), |x| x);
        assert!(out.is_empty());
    }
    #[test]
    fn streaming_bounds_lazy_inputs_and_reordered_results() {
        let sched = Scheduler::new(SchedulerConfig::new(2, 4));
        let produced = AtomicUsize::new(0);
        let (release_tx, release_rx) = crossbeam_channel::bounded(1);
        let ahead = AtomicUsize::new(0);
        let mut consumed = 0;
        let mut bytes = 0;
        // 512 MiB of payload must not be resident together. The first item
        // waits for the other five window slots, forcing the reorder buffer.
        let items = (0..512).map(|i| {
            produced.fetch_add(1, Ordering::SeqCst);
            (i, vec![i as u8; 1024 * 1024])
        });
        sched.run_streaming(items, |(i, payload)| {
            if i == 0 {
                release_rx.recv().unwrap();
            } else if ahead.fetch_add(1, Ordering::SeqCst) == 4 {
                release_tx.send(()).unwrap();
            }
            payload
        }, |i, payload| {
            assert_eq!(i, consumed);
            assert!(produced.load(Ordering::SeqCst) - consumed <= 6);
            assert!(payload.iter().all(|&b| b == i as u8));
            bytes += payload.len();
            consumed += 1;
            Ok::<_, ()>(())
        }).unwrap();
        assert_eq!(consumed, 512);
        assert_eq!(bytes, 512 * 1024 * 1024);
    }

    #[test]
    fn streaming_sink_error_cancels_lazy_ingress() {
        let produced = AtomicUsize::new(0);
        let items = (0..10_000).map(|i| {
            produced.fetch_add(1, Ordering::SeqCst);
            i
        });
        let result = Scheduler::new(SchedulerConfig::new(1, 4))
            .run_streaming(items, |i| i, |_, _| Err("disk full"));
        assert_eq!(result, Err("disk full"));
        assert!(produced.load(Ordering::SeqCst) <= 5);
    }

    #[test]
    fn streaming_worker_panic_cancels_without_deadlock() {
        let result = std::panic::catch_unwind(|| {
            Scheduler::new(SchedulerConfig::new(1, 4)).run_streaming(
                0..10_000,
                |i| { assert_ne!(i, 0, "worker panic"); i },
                |_, _| Ok::<_, ()>(()),
            )
        });
        assert!(result.is_err());
    }

    #[test]
    fn streaming_accepts_borrowed_closures_and_empty_input() {
        let offset = 42;
        let mut out = Vec::new();
        let sched = Scheduler::new(SchedulerConfig::new(1, 2));
        sched.run_streaming(0..4, |i| i + offset, |i, value| {
            out.push((i, value));
            Ok::<_, ()>(())
        }).unwrap();
        assert_eq!(out, vec![(0, 42), (1, 43), (2, 44), (3, 45)]);
        sched.run_streaming(std::iter::empty::<()>(), |_| (), |_, _| {
            panic!("empty stream called sink");
            #[allow(unreachable_code)] Ok::<_, ()>(())
        }).unwrap();
    }

}
