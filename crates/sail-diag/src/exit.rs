#![forbid(unsafe_code)]
//! Process exit codes for the Sail compiler driver (Lane 38 consumer).
//!
//! Mapping from diagnostics:
//! - [`ErrorKind::BackendLimit`](crate::ErrorKind::BackendLimit) → [`ExitCode::BackendLimit`] (4)
//! - [`ErrorKind::Internal`](crate::ErrorKind::Internal) → [`ExitCode::Internal`] (101)
//! - `--explain` / success → 0; CLI misuse → 2; I/O failure → 3.

use crate::{ErrorKind, SailError};
use std::fmt;

/// Process exit code. Use [`ExitCode::as_i32`] at `main`'s edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExitCode {
    /// Compilation (and `--explain`) succeeded.
    Success,
    /// Generic compile error (diagnostics already emitted).
    CompileError,
    /// CLI misuse: bad flags, bad `--explain` code, wrong arity.
    Usage,
    /// File not found / unreadable / unwritable.
    Io,
    /// Known Lean-backend limitation (`ErrorKind::BackendLimit`).
    BackendLimit,
    /// Violated compiler invariant (`ErrorKind::Internal`).
    Internal,
}

impl ExitCode {
    /// The integer for `std::process::exit`.
    #[must_use]
    pub fn as_i32(self) -> i32 {
        match self {
            ExitCode::Success => 0,
            ExitCode::CompileError => 1,
            ExitCode::Usage => 2,
            ExitCode::Io => 3,
            ExitCode::BackendLimit => 4,
            ExitCode::Internal => 101,
        }
    }

    /// Exit code for one emitted error.
    #[must_use]
    pub fn for_error(err: &SailError) -> ExitCode {
        ExitCode::for_kind(err.code.entry().kind)
    }

    /// Exit code for an error class.
    #[must_use]
    pub fn for_kind(kind: ErrorKind) -> ExitCode {
        match kind {
            ErrorKind::BackendLimit => ExitCode::BackendLimit,
            ErrorKind::Internal => ExitCode::Internal,
        }
    }

    /// Worst (highest-priority) code for a batch of errors: any
    /// [`ExitCode::Internal`] wins over [`ExitCode::BackendLimit`], which
    /// wins over [`ExitCode::CompileError`]; empty batch → [`ExitCode::Success`].
    #[must_use]
    pub fn for_errors(errors: &[SailError]) -> ExitCode {
        let mut worst = ExitCode::Success;
        for err in errors {
            let next = ExitCode::for_error(err);
            worst = match (worst, next) {
                (ExitCode::Internal, _) | (_, ExitCode::Internal) => ExitCode::Internal,
                (ExitCode::BackendLimit, _) | (_, ExitCode::BackendLimit) => ExitCode::BackendLimit,
                _ => ExitCode::CompileError,
            };
        }
        worst
    }
}

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({:?})", self.as_i32(), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiagCode, Pass, Span};

    fn err(code: DiagCode) -> SailError {
        SailError::new(Span::dummy(), Pass::LeanPrinter, code, "m", "h")
    }

    #[test]
    fn exit_numbers_are_stable() {
        assert_eq!(ExitCode::Success.as_i32(), 0);
        assert_eq!(ExitCode::CompileError.as_i32(), 1);
        assert_eq!(ExitCode::Usage.as_i32(), 2);
        assert_eq!(ExitCode::Io.as_i32(), 3);
        assert_eq!(ExitCode::BackendLimit.as_i32(), 4);
        assert_eq!(ExitCode::Internal.as_i32(), 101);
    }

    #[test]
    fn backend_limit_maps_to_4_and_internal_to_101() {
        assert_eq!(
            ExitCode::for_error(&err(DiagCode::NexpNotTranslatable)),
            ExitCode::BackendLimit
        );
        assert_eq!(
            ExitCode::for_error(&err(DiagCode::DefsRecUnreachable)),
            ExitCode::Internal
        );
    }

    #[test]
    fn batch_picks_worst_code() {
        assert_eq!(ExitCode::for_errors(&[]), ExitCode::Success);
        assert_eq!(
            ExitCode::for_errors(&[err(DiagCode::NexpNotTranslatable)]),
            ExitCode::BackendLimit
        );
        assert_eq!(
            ExitCode::for_errors(&[
                err(DiagCode::NexpNotTranslatable),
                err(DiagCode::DefsRecUnreachable),
            ]),
            ExitCode::Internal
        );
    }
}
