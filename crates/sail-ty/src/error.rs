#![forbid(unsafe_code)]
//! Error type with spans.

/// Byte span in the source file. `Span::DUMMY` for synthesized nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub file: u32,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const DUMMY: Self = Self {
        file: 0,
        start: 0,
        end: 0,
    };

    #[must_use]
    pub fn new(file: u32, start: u32, end: u32) -> Self {
        Self { file, start, end }
    }
}

/// Typechecker error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeError {
    pub span: Span,
    pub kind: TypeErrorKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeErrorKind {
    KindMismatch {
        expected: String,
        found: String,
    },
    UnboundVar(String),
    Mismatch {
        expected: String,
        found: String,
    },
    ConstraintNotProved(String),
    NegativeBits(i128),
    ArityMismatch {
        expected: usize,
        found: usize,
    },
    QuantError(String),
    FuelExhausted,
    RegisterError(String),
}

impl TypeError {
    #[must_use]
    pub fn kind_mismatch(
        span: Span,
        expected: crate::kind::Kind,
        found: crate::kind::Kind,
    ) -> Self {
        Self {
            span,
            kind: TypeErrorKind::KindMismatch {
                expected: expected.name().to_string(),
                found: found.name().to_string(),
            },
        }
    }

    #[must_use]
    pub fn unbound_var(span: Span, var: String) -> Self {
        Self {
            span,
            kind: TypeErrorKind::UnboundVar(var),
        }
    }

    #[must_use]
    pub fn mismatch(span: Span, expected: String, found: String) -> Self {
        Self {
            span,
            kind: TypeErrorKind::Mismatch { expected, found },
        }
    }

    #[must_use]
    pub fn constraint_not_proved(span: Span, nc: impl std::fmt::Display) -> Self {
        Self {
            span,
            kind: TypeErrorKind::ConstraintNotProved(nc.to_string()),
        }
    }

    #[must_use]
    pub fn negative_bits(span: Span, len: i128) -> Self {
        Self {
            span,
            kind: TypeErrorKind::NegativeBits(len),
        }
    }

    #[must_use]
    pub fn fuel_exhausted(span: Span) -> Self {
        Self {
            span,
            kind: TypeErrorKind::FuelExhausted,
        }
    }
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            TypeErrorKind::KindMismatch { expected, found } => write!(
                f,
                "kind mismatch at {:?}: expected {expected}, found {found}",
                self.span
            ),
            TypeErrorKind::UnboundVar(v) => {
                write!(f, "unbound variable '{v}' at {:?}", self.span)
            }
            TypeErrorKind::Mismatch { expected, found } => write!(
                f,
                "type mismatch at {:?}: expected {expected}, found {found}",
                self.span
            ),
            TypeErrorKind::ConstraintNotProved(nc) => write!(
                f,
                "constraint not proved at {:?}: {nc}",
                self.span
            ),
            TypeErrorKind::NegativeBits(n) => {
                write!(f, "negative bitvector length {n} at {:?}", self.span)
            }
            TypeErrorKind::ArityMismatch { expected, found } => write!(
                f,
                "arity mismatch at {:?}: expected {expected}, found {found}",
                self.span
            ),
            TypeErrorKind::QuantError(msg) => {
                write!(f, "quantifier error at {:?}: {msg}", self.span)
            }
            TypeErrorKind::FuelExhausted => {
                write!(f, "typechecker fuel exhausted at {:?}", self.span)
            }
            TypeErrorKind::RegisterError(msg) => {
                write!(f, "register error at {:?}: {msg}", self.span)
            }
        }
    }
}

impl std::error::Error for TypeError {}

pub type TypeResult<T> = Result<T, TypeError>;
