#![forbid(unsafe_code)]
//! Streaming Lean printer for the pure-Rust Sail compiler.
//!
//! Design notes (low-RAM + multithreaded):
//! - Everything writes incrementally into a [`core::fmt::Write`] sink
//!   (file buffer, `String`, …). No whole-program `Document` tree is ever
//!   built, so peak RAM stays O(largest single definition).
//! - The printer holds only shared configuration (`Options`) behind `Arc`,
//!   therefore `LeanPrinter: Send + Sync + Clone` and definitions can be
//!   rendered on multiple threads (see [`LeanPrinter::print_defs_parallel`]).
//!   Each thread renders one definition into its own small `String` buffer
//!   and the caller streams the buffers out in order.
//!
//! ## Monad mapping (documented stub rules)
//!
//! The OCaml backend (`sail_lean_backend/pretty_print_lean.ml`, read as
//! reference) maps Sail effects to Lean monads. This crate implements the
//! same mapping as explicit, documented rules for function signatures:
//!
//! | Sail effect | Lean skeleton |
//! |---|---|
//! | pure (`Pure`) | `def f (args) : Ret := <body>` — plain value, no monad |
//! | stateful / register / exception-free io (`Stateful`) | `def f (args) : SailM Ret := do <body>` — `SailM` is the Sail state monad with `readReg`/`writeReg` |
//! | early return (`EarlyReturn`) | `def f (args) : SailME Ret := do <body>` — `SailME` = `ExceptT … SailM`, `early_return x` becomes `SailME.throw x` |
//!
//! Loop idioms (stub rule, body printing is out of scope for lane 10):
//! - `while#` → Lean `while … do`, `until#` → `repeat … until`,
//!   `foreach#` → `for x in [from:to:step]i do`.
//! - Bitvector case-split on a scrutinee of type `BitVec n` uses
//!   `match_bv …` (bit-indexed patterns), otherwise plain `match …`.
//!
//! Anything outside the supported subset is a hard [`Error::Unsupported`]
//! carrying a [`Span`]. There is deliberately **no** silent `sorry` fallback.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

pub mod conv;

/// Source position for hard errors: the canonical `sail-ast` span
/// (Lane 61 unification of MIGRATION.md #28; previously a line/col-only
/// local copy, removed).
pub use sail_ast::{FileId, Span};

/// Printer failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Underlying sink failed.
    Fmt(String),
    /// Construct is not translatable; carries source span. Never `sorry`.
    Unsupported { msg: String, span: Span },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fmt(e) => write!(f, "lean printer IO error: {e}"),
            Self::Unsupported { msg, span } => {
                write!(f, "cannot translate to Lean at {span}: {msg}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<fmt::Error> for Error {
    fn from(_: fmt::Error) -> Self {
        Self::Fmt("fmt::Write failed".to_string())
    }
}

/// Size expression (`n` in `BitVec n`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nexp {
    /// Numeric constant, e.g. `8`.
    Const(i128),
    /// Size variable, e.g. `n`.
    Var(String),
    /// `a + b`.
    Sum(Box<Nexp>, Box<Nexp>),
    /// `a - b`.
    Minus(Box<Nexp>, Box<Nexp>),
    /// `a * b`.
    Times(Box<Nexp>, Box<Nexp>),
    /// `-a`.
    Neg(Box<Nexp>),
    /// `2 ^ a`.
    Pow2(Box<Nexp>),
}

/// Sail type subset supported by the streaming printer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typ {
    Unit,
    Int,
    Nat,
    Bool,
    String,
    /// `BitVec 1` for a single bit, `(BitVec n)` otherwise.
    Bit(String),
    BitVec(Box<Nexp>),
    Option(Box<Typ>),
    List(Box<Typ>),
    Result(Box<Typ>, Box<Typ>),
    Tuple(Vec<Typ>),
    /// `(Vector elem n)`.
    Vector(Box<Typ>, Box<Nexp>),
    /// Type variable / kid, e.g. `'n`.
    Var(String),
    /// Named type application, e.g. `mytype a b`.
    Named {
        name: String,
        args: Vec<TypArg>,
        span: Span,
    },
}

/// Type argument for [`Typ::Named`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypArg {
    Typ(Typ),
    Nexp(Nexp),
}

/// Sail literal subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lit {
    Unit,
    True,
    False,
    Num(i128),
    /// Hex bitvector, e.g. `0xFF#8`. `width`: bit-width for `#n` suffix.
    Hex {
        digits: String,
        width: Option<u32>,
    },
    /// Binary bitvector, e.g. `0b101#3`. Single bits print as `0`/`1`.
    Bin { bits: String, width: Option<u32> },
    Str(String),
}

/// Sail effect class driving the monad mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effect {
    /// Pure value: no monad.
    #[default]
    Pure,
    /// Stateful (registers / `SailM … do`).
    Stateful,
    /// Early return (`SailME` / `ExceptT` over `SailM`).
    EarlyReturn,
}

/// Simple function signature: `def name (args) : Ret :=`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunSig {
    pub name: String,
    pub args: Vec<(String, Typ)>,
    pub ret: Typ,
    pub effect: Effect,
    pub span: Span,
}

/// Printer options (mirrors `-lean-line-width`, `noncomputable/partial`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Soft line-width hint (currently informational; printer emits
    /// short single-line forms and never builds a reflow tree).
    pub line_width: usize,
    /// Functions emitted with a `noncomputable` prefix.
    pub noncomputable: BTreeSet<String>,
    /// Functions emitted with a `partial` prefix.
    pub partial: BTreeSet<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            line_width: 100,
            noncomputable: BTreeSet::new(),
            partial: BTreeSet::new(),
        }
    }
}

/// Streaming Lean printer. Cheap to clone, `Send + Sync`.
#[derive(Debug, Clone)]
pub struct LeanPrinter {
    opts: Arc<Options>,
}

impl LeanPrinter {
    #[must_use]
    pub fn new(opts: Options) -> Self {
        Self {
            opts: Arc::new(opts),
        }
    }

    #[must_use]
    pub fn with_default() -> Self {
        Self::new(Options::default())
    }

    #[must_use]
    pub fn options(&self) -> &Options {
        &self.opts
    }

    /// Print a type expression, streaming into `out`.
    ///
    /// Mapping (from `pretty_print_lean.ml#doc_typ`, re-implemented):
    /// `unit→Unit`, `int→Int`, `nat→Nat`, `bool→Bool`, `string→String`,
    /// `bit→(BitVec 1)`, `bitvector/bits n→(BitVec n)`, `option→(Option t)`,
    /// `list→(List t)`, `result→(Result e a)`, tuples → `(a × b)`.
    pub fn print_typ<W: fmt::Write>(&self, out: &mut W, ty: &Typ) -> Result<(), Error> {
        match ty {
            Typ::Unit => out.write_str("Unit")?,
            Typ::Int => out.write_str("Int")?,
            Typ::Nat => out.write_str("Nat")?,
            Typ::Bool => out.write_str("Bool")?,
            Typ::String => out.write_str("String")?,
            Typ::Bit(_) => out.write_str("(BitVec 1)")?,
            Typ::BitVec(n) => {
                out.write_str("(BitVec ")?;
                Self::print_nexp_inner(out, n)?;
                out.write_str(")")?;
            }
            Typ::Option(t) => {
                out.write_str("(Option ")?;
                self.print_typ(out, t)?;
                out.write_str(")")?;
            }
            Typ::List(t) => {
                out.write_str("(List ")?;
                self.print_typ(out, t)?;
                out.write_str(")")?;
            }
            Typ::Result(e, a) => {
                out.write_str("(Result ")?;
                self.print_typ(out, e)?;
                out.write_char(' ')?;
                self.print_typ(out, a)?;
                out.write_str(")")?;
            }
            Typ::Tuple(ts) => {
                out.write_char('(')?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        out.write_str(" × ")?;
                    }
                    self.print_typ(out, t)?;
                }
                out.write_char(')')?;
            }
            Typ::Vector(elem, n) => {
                out.write_str("(Vector ")?;
                self.print_typ(out, elem)?;
                out.write_char(' ')?;
                Self::print_nexp_inner(out, n)?;
                out.write_str(")")?;
            }
            Typ::Var(v) => out.write_str(v)?,
            Typ::Named { name, args, span } => {
                if name.is_empty() {
                    return Err(Error::Unsupported {
                        msg: "anonymous type application is not translatable".to_string(),
                        span: *span,
                    });
                }
                if args.is_empty() {
                    write!(out, "{name}")?;
                } else {
                    out.write_char('(')?;
                    write!(out, "{name}")?;
                    for a in args {
                        out.write_char(' ')?;
                        self.print_typ_arg(out, a)?;
                    }
                    out.write_char(')')?;
                }
            }
        }
        Ok(())
    }

    fn print_typ_arg<W: fmt::Write>(&self, out: &mut W, arg: &TypArg) -> Result<(), Error> {
        match arg {
            TypArg::Typ(t) => self.print_typ(out, t),
            TypArg::Nexp(n) => Self::print_nexp_inner(out, n),
        }
    }

    /// Print a size expression with correct precedence, streaming.
    /// Grammar: `+,-` (lowest) > `*,/, %` > unary `-` > `2 ^` > atom.
    pub fn print_nexp<W: fmt::Write>(out: &mut W, n: &Nexp) -> Result<(), Error> {
        Self::print_nexp_inner(out, n)
    }

    fn print_nexp_inner<W: fmt::Write>(out: &mut W, n: &Nexp) -> Result<(), Error> {
        Self::print_nexp_sum(out, n)
    }

    fn print_nexp_sum<W: fmt::Write>(out: &mut W, n: &Nexp) -> Result<(), Error> {
        match n {
            Nexp::Sum(a, b) => {
                Self::print_nexp_sum(out, a)?;
                out.write_str(" + ")?;
                Self::print_nexp_product(out, b)?;
                Ok(())
            }
            Nexp::Minus(a, b) => {
                Self::print_nexp_sum(out, a)?;
                out.write_str(" - ")?;
                Self::print_nexp_product(out, b)?;
                Ok(())
            }
            _ => Self::print_nexp_product(out, n),
        }
    }

    fn print_nexp_product<W: fmt::Write>(out: &mut W, n: &Nexp) -> Result<(), Error> {
        match n {
            Nexp::Times(a, b) => {
                Self::print_nexp_product(out, a)?;
                out.write_str(" * ")?;
                Self::print_nexp_unary(out, b)?;
                Ok(())
            }
            _ => Self::print_nexp_unary(out, n),
        }
    }

    fn print_nexp_unary<W: fmt::Write>(out: &mut W, n: &Nexp) -> Result<(), Error> {
        match n {
            Nexp::Neg(a) => {
                out.write_str("(- ")?;
                Self::print_nexp_unary(out, a)?;
                out.write_char(')')?;
                Ok(())
            }
            _ => Self::print_nexp_pow(out, n),
        }
    }

    fn print_nexp_pow<W: fmt::Write>(out: &mut W, n: &Nexp) -> Result<(), Error> {
        match n {
            Nexp::Pow2(e) => {
                out.write_str("2 ^ ")?;
                Self::print_nexp_pow(out, e)?;
                Ok(())
            }
            _ => Self::print_nexp_atom(out, n),
        }
    }

    fn print_nexp_atom<W: fmt::Write>(out: &mut W, n: &Nexp) -> Result<(), Error> {
        match n {
            Nexp::Const(i) => {
                if *i < 0 {
                    write!(out, "({i})")?;
                } else {
                    write!(out, "{i}")?;
                }
                Ok(())
            }
            Nexp::Var(v) => {
                out.write_str(v)?;
                Ok(())
            }
            Nexp::Sum(..) | Nexp::Minus(..) | Nexp::Times(..) | Nexp::Neg(..) | Nexp::Pow2(..) => {
                out.write_char('(')?;
                Self::print_nexp_inner(out, n)?;
                out.write_char(')')?;
                Ok(())
            }
        }
    }

    /// Print a literal, streaming. Mirrors `doc_lit` in the OCaml backend:
    /// empty hex/bin → `BitVec.nil`, single bits → `0`/`1` (with `#w`
    /// suffix when a width is known), otherwise `0x…`/`0b…` + `#w`.
    pub fn print_lit<W: fmt::Write>(&self, out: &mut W, lit: &Lit) -> Result<(), Error> {
        match lit {
            Lit::Unit => out.write_str("()")?,
            Lit::True => out.write_str("true")?,
            Lit::False => out.write_str("false")?,
            Lit::Num(i) => {
                if *i < 0 {
                    write!(out, "({i})")?;
                } else {
                    write!(out, "{i}")?;
                }
            }
            Lit::Hex { digits, width } => {
                if digits.is_empty() {
                    out.write_str("BitVec.nil")?;
                } else {
                    write!(out, "0x{}", digits.to_uppercase())?;
                    if let Some(w) = width {
                        write!(out, "#{w}")?;
                    }
                }
            }
            Lit::Bin { bits, width } => {
                if bits.is_empty() {
                    out.write_str("BitVec.nil")?;
                } else if bits.len() == 1 {
                    out.write_str(bits.as_str())?;
                    if let Some(w) = width {
                        write!(out, "#{w}")?;
                    }
                } else {
                    write!(out, "0b{bits}")?;
                    if let Some(w) = width {
                        write!(out, "#{w}")?;
                    }
                }
            }
            Lit::Str(s) => {
                out.write_char('"')?;
                for c in s.chars() {
                    match c {
                        '"' => out.write_str("\\\"")?,
                        '\\' => out.write_str("\\\\")?,
                        '\n' => out.write_str("\\n")?,
                        c => out.write_char(c)?,
                    }
                }
                out.write_char('"')?;
            }
        }
        Ok(())
    }

    /// Render the Lean monad for an effect (`Pure` → no monad).
    pub fn monad_of(&self, effect: Effect) -> Option<&'static str> {
        match effect {
            Effect::Pure => None,
            Effect::Stateful => Some("SailM"),
            Effect::EarlyReturn => Some("SailME"),
        }
    }

    /// Print a function signature header:
    /// `[noncomputable ][partial ]def name (arg : Ty)… : Ret :=`.
    /// The trailing body placeholder is emitted by [`Self::print_fun_sig`]'s
    /// caller (or use `print_fun_stub` for a complete `sorry`-free stub —
    /// actually a hard error is raised when no body printer is available;
    /// stubs carry `panic` text only in tests via `print_fun_stub`).
    pub fn print_sig_header<W: fmt::Write>(
        &self,
        out: &mut W,
        sig: &FunSig,
    ) -> Result<(), Error> {
        if sig.name.is_empty() {
            return Err(Error::Unsupported {
                msg: "anonymous function is not translatable".to_string(),
                span: sig.span,
            });
        }
        if self.opts.noncomputable.contains(&sig.name) {
            out.write_str("noncomputable ")?;
        }
        if self.opts.partial.contains(&sig.name) {
            out.write_str("partial ")?;
        }
        write!(out, "def {}", sanitize_ident(&sig.name))?;
        for (name, ty) in &sig.args {
            write!(out, " ({} : ", sanitize_ident(name))?;
            self.print_typ(out, ty)?;
            out.write_char(')')?;
        }
        out.write_str(" : ")?;
        if let Some(monad) = self.monad_of(sig.effect) {
            write!(out, "{monad} ")?;
        }
        self.print_typ(out, &sig.ret)?;
        out.write_str(" :=")?;
        Ok(())
    }

    /// Print a complete stub definition with a `do`-block opener for
    /// monadic effects and a hard-error body marker when `body` is `None`.
    ///
    /// `body: None` models "body not translatable in this lane": instead of
    /// emitting a silent `sorry`, the printer returns
    /// [`Error::Unsupported`] with the function [`Span`].
    pub fn print_fun_stub<W: fmt::Write>(
        &self,
        out: &mut W,
        sig: &FunSig,
        body: Option<&str>,
    ) -> Result<(), Error> {
        self.print_sig_header(out, sig)?;
        match self.monad_of(sig.effect) {
            None => {
                // Pure: `def f (…) : Ret := <body>`.
                out.write_char(' ')?;
                match body {
                    Some(b) => out.write_str(b)?,
                    None => {
                        return Err(Error::Unsupported {
                            msg: format!(
                                "function `{}` body is not translatable (no silent sorry)",
                                sig.name
                            ),
                            span: sig.span,
                        });
                    }
                }
            }
            Some(_) => {
                // Monadic: `… := do\n  <body>`.
                out.write_str(" do\n  ")?;
                match body {
                    Some(b) => out.write_str(b)?,
                    None => {
                        return Err(Error::Unsupported {
                            msg: format!(
                                "function `{}` body is not translatable (no silent sorry)",
                                sig.name
                            ),
                            span: sig.span,
                        });
                    }
                }
            }
        }
        out.write_char('\n')?;
        Ok(())
    }

    /// Render one definition to a fresh `String` (small per-def buffer).
    pub fn render_fun_stub(&self, sig: &FunSig, body: Option<&str>) -> Result<String, Error> {
        let mut buf = String::new();
        self.print_fun_stub(&mut buf, sig, body)?;
        Ok(buf)
    }

    /// Multithreaded rendering: each signature is rendered on its own
    /// thread (via `std::thread::scope`) into a per-def `String`, then
    /// streamed out in input order. Peak RAM stays O(largest def ×
    /// parallelism); no whole-program document tree is built.
    ///
    /// `bodies[i]` is the body text (or `None` for untranslatable) for
    /// `sigs[i]`. Returns the first hard error encountered, if any.
    pub fn print_defs_parallel<W: fmt::Write + Send>(
        &self,
        out: &mut W,
        sigs: &[FunSig],
        bodies: &[Option<String>],
    ) -> Result<(), Error> {
        assert_eq!(
            sigs.len(),
            bodies.len(),
            "sigs and bodies must have equal length"
        );
        if sigs.is_empty() {
            return Ok(());
        }
        let n = sigs.len();
        let mut rendered: Vec<Option<Result<String, Error>>> = Vec::with_capacity(n);
        rendered.resize_with(n, || None);
        std::thread::scope(|s| {
            let chunks: Vec<_> = rendered
                .iter_mut()
                .enumerate()
                .map(|(i, slot)| {
                    let printer = self.clone();
                    let sig = &sigs[i];
                    let body = &bodies[i];
                    s.spawn(move || {
                        *slot = Some(printer.render_fun_stub(sig, body.as_deref()));
                    })
                })
                .collect();
            let _ = chunks;
        });
        for slot in rendered {
            match slot {
                Some(Ok(text)) => out.write_str(&text)?,
                Some(Err(e)) => return Err(e),
                None => {
                    return Err(Error::Fmt("worker thread failed to render".to_string()));
                }
            }
        }
        Ok(())
    }

    /// Emit the `SailM`/`SailME` prelude stub referenced by monadic
    /// signatures (documents the expected runtime, mirroring the OCaml
    /// backend's `SailM`/`ExceptM` usage).
    pub fn print_prelude<W: fmt::Write>(&self, out: &mut W) -> Result<(), Error> {
        out.write_str(
            "-- Sail monad stubs (see lane-10 monad mapping).\n\
             -- Pure Sail functions are plain Lean values.\n\
             -- Stateful Sail functions live in `SailM` (do-notation, `readReg`/`writeReg`).\n\
             -- Early-return Sail functions live in `SailME` (`ExceptT` over `SailM`;\n\
             -- `early_return x` maps to `SailME.throw x`).\n\
             -- Loops: `while#` → `while … do`, `until#` → `repeat … until`,\n\
             -- `foreach#` → `for x in [from:to:step]i do`.\n\
             -- `match_bv` is used for `BitVec` case-splits, otherwise `match`.\n\
             abbrev SailM (α : Type) : Type := IO α\n\
             abbrev SailME (α : Type) : Type := ExceptT String IO α\n",
        )?;
        Ok(())
    }
}

/// Lean-identifier sanitizer: keywords and illegal characters are suffixed
/// or replaced so output always parses.
fn sanitize_ident(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "def", "match", "where", "with", "if", "then", "else", "for", "in", "do", "let", "mut",
        "return", "import", "open", "namespace", "section", "end", "structure", "inductive",
        "abbrev", "theorem", "lemma", "example", "sorry", "admit",
    ];
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if c.is_alphanumeric() || c == '_' || (i > 0 && (c == '\'' || c == '!')) {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out.starts_with(char::is_numeric) {
        out.insert(0, '_');
    }
    if KEYWORDS.contains(&out.as_str()) {
        out.push('\'');
    }
    out
}

#[cfg(test)]
mod tests;
