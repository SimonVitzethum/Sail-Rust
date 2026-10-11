//! Persistent Z3 subprocess per checked-out solver. No native dependency. Each
//! query resets state; a parse error, timeout or broken process never proves a
//! goal. Children inherit the compiler's systemd memory/swap limits.
use crate::{Constraint, Solver, SolverQuery, SolverVerdict, StubSolver};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

type Job = (String, u64, mpsc::Sender<std::io::Result<SolverVerdict>>);

struct Process {
    child: Child,
    jobs: Option<mpsc::SyncSender<Job>>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.jobs.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Process {
    fn start() -> std::io::Result<Self> {
        let mut child = Command::new("z3")
            .args(["-in", "-smt2"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut input = child.stdin.take().expect("piped Z3 stdin");
        let mut output = BufReader::new(child.stdout.take().expect("piped Z3 stdout"));
        let (jobs, receiver) = mpsc::sync_channel::<Job>(1);
        let worker = std::thread::Builder::new()
            .name("sail-z3-io".into())
            .spawn(move || {
                while let Ok((script, timeout, response)) = receiver.recv() {
                    let result = Self::transact(&mut input, &mut output, &script, timeout);
                    let failed = result.is_err();
                    let _ = response.send(result);
                    if failed {
                        break;
                    }
                }
            });
        match worker {
            Ok(worker) => Ok(Self {
                child,
                jobs: Some(jobs),
                worker: Some(worker),
            }),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }
    fn prove(&mut self, script: &str, timeout_ms: u64) -> std::io::Result<SolverVerdict> {
        let timeout_ms = timeout_ms.clamp(1, 60_000);
        let (response, receiver) = mpsc::channel();
        self.jobs
            .as_ref()
            .ok_or(std::io::ErrorKind::BrokenPipe)?
            .send((script.to_owned(), timeout_ms, response))
            .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
        receiver
            .recv_timeout(Duration::from_millis(timeout_ms + 1000))
            .map_err(|_| std::io::ErrorKind::TimedOut)?
    }
    fn transact(
        input: &mut ChildStdin,
        output: &mut BufReader<ChildStdout>,
        script: &str,
        timeout_ms: u64,
    ) -> std::io::Result<SolverVerdict> {
        writeln!(input,"(reset)\n(set-option :print-success false)\n(set-option :timeout {})\n{}(check-sat)\n(echo \"sail_query_done\")",timeout_ms,script)?;
        input.flush()?;
        let mut verdict = SolverVerdict::Unknown;
        let mut invalid = false;
        let mut seen = false;
        loop {
            let mut line = String::new();
            if output.read_line(&mut line)? == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            match line.trim() {
                "sail_query_done" => {
                    return Ok(if invalid || !seen {
                        SolverVerdict::Unknown
                    } else {
                        verdict
                    })
                }
                "unsat" if !seen => {
                    verdict = SolverVerdict::Valid;
                    seen = true;
                }
                "sat" | "unknown" if !seen => {
                    seen = true;
                }
                _ => invalid = true,
            }
        }
    }
}
pub struct SmtLibSolver {
    process: Mutex<Option<Process>>,
    timeout_ms: u64,
}
impl Default for SmtLibSolver {
    fn default() -> Self {
        Self::new()
    }
}
impl SmtLibSolver {
    pub fn new() -> Self {
        Self {
            process: Mutex::new(None),
            timeout_ms: 3000,
        }
    }
}
impl Solver for SmtLibSolver {
    fn prove(&self, query: &SolverQuery) -> SolverVerdict {
        let Constraint::SmtLib(query_body) = &query.constraint else {
            return StubSolver::new().prove(query);
        };
        let Ok(mut slot) = self.process.lock() else {
            return SolverVerdict::Unknown;
        };
        if slot.is_none() {
            *slot = Process::start().ok();
        }
        let Some(process) = slot.as_mut() else {
            return SolverVerdict::Unknown;
        };
        match process.prove(
            query_body.script(),
            query.timeout_ms.unwrap_or(self.timeout_ms),
        ) {
            Ok(verdict) => verdict,
            Err(_) => {
                *slot = None;
                SolverVerdict::Unknown
            }
        }
    }
    fn name(&self) -> &'static str {
        "z3-smtlib"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_process_resets_each_query_and_rejects_parse_errors() {
        let mut process =
            Process::start().expect("Z3 executable required for canonical typing tests");
        assert_eq!(
            process.prove("(assert false)\n", 1000).unwrap(),
            SolverVerdict::Valid
        );
        assert_eq!(
            process.prove("(assert true)\n", 1000).unwrap(),
            SolverVerdict::Unknown
        );
        assert_eq!(
            process
                .prove("(assert false)\n(no-such-command)\n", 1000)
                .unwrap(),
            SolverVerdict::Unknown
        );
        assert_eq!(
            process.prove("(assert false)\n", 1000).unwrap(),
            SolverVerdict::Valid
        );
    }
}
