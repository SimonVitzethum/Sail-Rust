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
//! ## Monad mapping (ports `doc_funcl_init`, `prepend_monad`, `doc_monad_abbrev`)
//!
//! The OCaml backend (`sail_lean_backend/pretty_print_lean.ml`, read as
//! reference) computes three independent predicates per function and maps
//! Sail effects to Lean monads from them:
//! - `is_monadic` (`not (Effects.function_is_pure …)`, ~line 1160) prefixes
//!   the return type with `SailM`;
//! - `early_return` (`has_early_return exp`, ~line 1161) selects the
//!   `ExceptM` layer (`SailME.throw` vs `throw`, `sailTryCatchE` vs
//!   `sailTryCatch`);
//! - `has_loop` (`has_loop exp`, ~line 1162) selects the `Id.run` frame.
//!
//! | is_monadic | early_return | has_loop | Lean signature |
//! |---|---|---|
//! | false | false | false | `def f (args) : Ret := <body>` — plain value |
//! | true | false | _ | `def f (args) : SailM Ret := do <body>` |
//! | true | true | _ | `def f (args) : SailM Ret := SailME.run do <body>` |
//! | false | false | true | `def f (args) : Ret := Id.run do <body>` |
//! | false | true | _ | `def f (args) : Ret := ExceptM.run do <body>` |
//!
//! The frame matrix itself is the single master in
//! [`sail_except::function_frame`]; [`LeanPrinter::function_frame_suffix`]
//! re-exports its tokens here so signatures and effect analysis cannot
//! drift apart. `let`-ascriptions follow `prepend_monad` (~line 725):
//! `(SailM, SailME, ExceptM, Id)` — see
//! [`LeanPrinter::print_ascribed_monad`].
//!
//! Loop idioms (stub rule, body printing is out of scope for lane 10):
//! - `while#` → Lean `while … do`, `until#` → `repeat … until`,
//!   `foreach#` → `for x in [from:to:step]i do`.
//! - Bitvector case-split on a scrutinee of type `BitVec n` uses
//!   `match_bv …` (bit-indexed patterns), otherwise plain `match …`.
//!
//! Anything outside the supported subset is a hard [`Error::Unsupported`]
//! carrying a [`Span`]. There is deliberately **no** silent `sorry` fallback.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

pub mod conv;
pub mod pats;

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
    /// Kind variable (`Nexp_var`, e.g. `'n`): prints with the OCaml
    /// `k_` prefix (`doc_kid`). Callers mapping `Nexp_id` here must pass
    /// the already-plain id text — the prefix applies uniformly.
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
    /// Kind variable (`Typ_var`, e.g. `'n`): prints with the OCaml `k_`
    /// prefix (`doc_kid`).
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
///
/// Coarse legacy label: the Lean-accurate form is the independent
/// `(is_monadic, early_return, has_loop)` triple on [`FunSig`] (OCaml
/// `doc_funcl_init` computes the three predicates independently, so e.g.
/// pure-with-early-return (`ExceptM.run`) has no single-enum spelling).
/// [`Effect::triple`] maps each variant to its `(is_monadic,
/// early_return)` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effect {
    /// Pure value: no monad.
    #[default]
    Pure,
    /// Stateful (registers / `SailM … do`).
    Stateful,
    /// Stateful with early return (`SailM … := SailME.run do`).
    EarlyReturn,
}

impl Effect {
    /// `(is_monadic, early_return)` pair for this coarse label.
    #[must_use]
    pub fn triple(self) -> (bool, bool) {
        match self {
            Self::Pure => (false, false),
            Self::Stateful => (true, false),
            Self::EarlyReturn => (true, true),
        }
    }
}

/// Function signature: `def name (args) : [SailM] Ret := [frame]`.
/// `is_monadic` / `early_return` / `has_loop` are the three independent
/// OCaml predicates (`doc_funcl_init`, ~lines 1160-1162); the emitted
/// return-type prefix and body frame follow the table in the crate docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunSig {
    pub name: String,
    pub args: Vec<(String, Typ)>,
    pub ret: Typ,
    /// `not (Effects.function_is_pure …)`: return type gains `SailM`.
    pub is_monadic: bool,
    /// `has_early_return exp`: body frame gains the `ExceptM` layer.
    pub early_return: bool,
    /// `has_loop exp`: pure bodies gain the `Id.run` frame.
    pub has_loop: bool,
    pub span: Span,
}

/// Printer options (mirrors `-lean-line-width`, `noncomputable/partial`,
/// plus the OCaml global context fragments `pretty_print_lean.ml` needs
/// for identifiers: `ctor_renames` (`compute_ctor_renames`), the enum
/// member set (`.` prefix in patterns), `defined_functions` +
/// `out_name_camel` (shadowing prefix in type positions)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Soft line-width hint (currently informational; printer emits
    /// short single-line forms and never builds a reflow tree).
    pub line_width: usize,
    /// Functions emitted with a `noncomputable` prefix.
    pub noncomputable: BTreeSet<String>,
    /// Functions emitted with a `partial` prefix.
    pub partial: BTreeSet<String>,
    /// Union-constructor renames (`compute_ctor_renames`,
    /// `pretty_print_lean.ml#1673-1686`): constructor name → owning
    /// type-name prefix. A renamed ctor prints as `Pref.Ctor`.
    pub ctor_renames: BTreeMap<String, String>,
    /// Enum member ids: patterns over them print with a leading `.`
    /// (`pretty_print_lean.ml#538`).
    pub enum_members: BTreeSet<String>,
    /// Function/type names visible as shadowing globals
    /// (`defined_functions`, `pretty_print_lean.ml#129-133`).
    pub defined_functions: BTreeSet<String>,
    /// Module prefix for the shadowing rule (`out_name_camel`).
    pub out_name_camel: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            line_width: 100,
            noncomputable: BTreeSet::new(),
            partial: BTreeSet::new(),
            ctor_renames: BTreeMap::new(),
            enum_members: BTreeSet::new(),
            defined_functions: BTreeSet::new(),
            out_name_camel: String::new(),
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
            Typ::Var(v) => out.write_str(&print_kid_name(v))?,
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
            Nexp::Const(i) => print_big_int(out, *i),
            Nexp::Var(v) => {
                out.write_str(&print_kid_name(v))?;
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

    /// Print a literal, streaming. Mirrors `doc_lit` in the OCaml backend
    /// (`pretty_print_lean.ml#397-415`):
    /// empty hex/bin → `BitVec.nil`, single bits → `0`/`1` (with `#w`
    /// suffix when a width is known and `width` is set), otherwise
    /// `0x…`/`0b…` + `#w`.
    ///
    /// `width` mirrors the OCaml `~width` flag: expressions print with
    /// `width:true` (`E_lit`, #861), patterns with `width:false`
    /// (`P_lit`, #525), so pattern literals never carry a `#w` suffix.
    /// Strings escape only `"` (`lean_escape_string`, #395).
    pub fn print_lit<W: fmt::Write>(&self, out: &mut W, lit: &Lit) -> Result<(), Error> {
        self.print_lit_with_width(out, lit, true)
    }

    /// [`Self::print_lit`] with an explicit OCaml `~width` flag.
    pub fn print_lit_with_width<W: fmt::Write>(
        &self,
        out: &mut W,
        lit: &Lit,
        width: bool,
    ) -> Result<(), Error> {
        match lit {
            Lit::Unit => out.write_str("()")?,
            Lit::True => out.write_str("true")?,
            Lit::False => out.write_str("false")?,
            Lit::Num(i) => print_big_int(out, *i)?,
            Lit::Hex { digits, width: w } => {
                if digits.is_empty() {
                    out.write_str("BitVec.nil")?;
                } else {
                    write!(out, "0x{}", digits.to_uppercase())?;
                    if width {
                        if let Some(w) = w {
                            write!(out, "#{w}")?;
                        }
                    }
                }
            }
            Lit::Bin { bits, width: w } => {
                if bits.is_empty() {
                    out.write_str("BitVec.nil")?;
                } else if bits.len() == 1 {
                    out.write_str(bits.as_str())?;
                    if width {
                        if let Some(w) = w {
                            write!(out, "#{w}")?;
                        }
                    }
                } else {
                    write!(out, "0b{bits}")?;
                    if width {
                        if let Some(w) = w {
                            write!(out, "#{w}")?;
                        }
                    }
                }
            }
            Lit::Str(s) => {
                out.write_char('"')?;
                for c in s.chars() {
                    match c {
                        '"' => out.write_str("\\\"")?,
                        c => out.write_char(c)?,
                    }
                }
                out.write_char('"')?;
            }
        }
        Ok(())
    }

    /// Render the Lean return-type monad for an effect label
    /// (`Pure` → no monad). Thin legacy wrapper over
    /// [`Self::ret_monad_of`]; prefer the `(is_monadic, early_return)`
    /// pair directly (a single enum cannot spell pure-with-early-return).
    pub fn monad_of(&self, effect: Effect) -> Option<&'static str> {
        Self::ret_monad_of(effect.triple().0)
    }

    /// Return-type monad (`doc_funcl_init`, ~line 1164): stateful functions
    /// return `SailM Ret`, pure ones the plain `Ret` — independently of
    /// `early_return` (a pure early return runs in `ExceptM`, selected by
    /// the frame, not the return type).
    pub fn ret_monad_of(is_monadic: bool) -> Option<&'static str> {
        if is_monadic { Some("SailM") } else { None }
    }

    /// Body-frame tokens appended after `def f … : T :=` (`dec_val_end`,
    /// ~lines 1167-1174): `SailME.run do` / `do` / `Id.run do` /
    /// `ExceptM.run do` / ``. Single master is
    /// [`sail_except::function_frame`]; this re-exports its suffix so the
    /// printer and the effect analysis share one matrix.
    pub fn function_frame_suffix(is_monadic: bool, early_return: bool, has_loop: bool) -> &'static str {
        sail_except::function_frame(is_monadic, early_return, has_loop).suffix()
    }

    /// `early_return x` combinator (`doc_exp`, ~lines 935-937):
    /// `SailME.throw` inside `SailM`, bare `throw` otherwise.
    pub fn throw_name(in_sail_monad: bool) -> &'static str {
        if in_sail_monad { "SailME.throw" } else { "throw" }
    }

    /// Monad ascription for a `let`-bound typed binding (`prepend_monad`,
    /// ~lines 725-730): `(in_sail_monad, in_except_monad)` selects
    /// `SailME <ret> <ascribed>` / `SailM <ascribed>` /
    /// `ExceptM <ret> <ascribed>` / `Id <ascribed>`. `except_ret` is `Some`
    /// exactly when the enclosing function has an early return, holding
    /// its original return type.
    pub fn print_ascribed_monad<W: fmt::Write>(
        &self,
        out: &mut W,
        in_sail_monad: bool,
        except_ret: Option<&Typ>,
        ascribed: &Typ,
    ) -> Result<(), Error> {
        match (in_sail_monad, except_ret) {
            (true, Some(ret)) => {
                out.write_str("SailME ")?;
                self.print_typ(out, ret)?;
                out.write_char(' ')?;
                self.print_typ(out, ascribed)?;
            }
            (true, None) => {
                out.write_str("SailM ")?;
                self.print_typ(out, ascribed)?;
            }
            (false, Some(ret)) => {
                out.write_str("ExceptM ")?;
                self.print_typ(out, ret)?;
                out.write_char(' ')?;
                self.print_typ(out, ascribed)?;
            }
            (false, None) => {
                out.write_str("Id ")?;
                self.print_typ(out, ascribed)?;
            }
        }
        Ok(())
    }

    /// Port of OCaml `doc_id_ctor` (`pretty_print_lean.ml#117-126`).
    /// With `ren_ctor`, union constructors listed in
    /// [`Options::ctor_renames`] print qualified as `Pref.Ctor`
    /// (`rename_ctor`, #109-115; [`Options::ctor_renames`] mirrors
    /// `compute_ctor_renames`, #1673-1686). Without it, the name prints
    /// sanitized. (OCaml `And_bool`/`Or_bool`/`Operator` spellings have
    /// no counterpart in this crate's plain-`String` ids; documented gap.)
    pub fn print_id_ctor<W: fmt::Write>(
        &self,
        out: &mut W,
        name: &str,
        ren_ctor: bool,
    ) -> Result<(), Error> {
        if ren_ctor {
            if let Some(pref) = self.opts.ctor_renames.get(name) {
                write!(out, "{}.{}", sanitize_ident(pref), sanitize_ident(name))?;
                return Ok(());
            }
        }
        out.write_str(&sanitize_ident(name))?;
        Ok(())
    }

    /// Port of OCaml `doc_id_typ_ctor` (`pretty_print_lean.ml#128-134`):
    /// names shadowed by [`Options::defined_functions`] gain an
    /// `Out.` prefix ([`Options::out_name_camel]). The
    /// `Env.lookup_id`-bound half of OCaml's `is_shadowed` has no type
    /// environment in this crate and is approximated by the
    /// `defined_functions` set alone (documented gap).
    pub fn print_id_typ_ctor<W: fmt::Write>(
        &self,
        out: &mut W,
        name: &str,
    ) -> Result<(), Error> {
        let fixed = sanitize_ident(name);
        if !self.opts.out_name_camel.is_empty() && self.opts.defined_functions.contains(name) {
            write!(out, "{}.{}", self.opts.out_name_camel, fixed)?;
        } else {
            out.write_str(&fixed)?;
        }
        Ok(())
    }

    /// Print a function signature header:
    /// `[noncomputable ][partial ]def name (arg : Ty)… : [SailM] Ret :=`.
    /// The body frame and body text are emitted by [`Self::print_fun_stub`]
    /// (or use `render_fun_stub` for a complete `sorry`-free stub —
    /// actually a hard error is raised when no body printer is available).
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
        if let Some(monad) = Self::ret_monad_of(sig.is_monadic) {
            write!(out, "{monad} ")?;
        }
        self.print_typ(out, &sig.ret)?;
        out.write_str(" :=")?;
        Ok(())
    }

    /// Print a complete stub definition with the OCaml body frame for
    /// the `(is_monadic, early_return, has_loop)` triple and a hard-error
    /// body marker when `body` is `None`.
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
        let frame = Self::function_frame_suffix(sig.is_monadic, sig.early_return, sig.has_loop);
        let text = match body {
            Some(b) => b,
            None => {
                return Err(Error::Unsupported {
                    msg: format!(
                        "function `{}` body is not translatable (no silent sorry)",
                        sig.name
                    ),
                    span: sig.span,
                });
            }
        };
        if frame.is_empty() {
            // Pure: `def f (…) : Ret := <body>`.
            out.write_char(' ')?;
            out.write_str(text)?;
        } else {
            // `… := <frame>\n  <body>` (`SailME.run do` / `do` /
            // `Id.run do` / `ExceptM.run do`).
            out.write_char(' ')?;
            out.write_str(frame)?;
            out.write_str("\n  ")?;
            out.write_str(text)?;
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

    /// Render using a bounded worker pool and write each result in input order.
    /// Additional rendered output is bounded by the scheduler credit window;
    /// the caller's signature/body slices remain resident. A slow first
    /// definition cannot accumulate all later rendered strings.
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
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
            .min(sigs.len().max(1));
        let scheduler = sail_shard::Scheduler::new(sail_shard::SchedulerConfig::new(8, workers));
        scheduler.run_streaming(sigs.iter().zip(bodies),
            |(sig, body)| self.render_fun_stub(sig, body.as_deref()),
            |_, rendered| { out.write_str(&rendered?)?; Ok(()) })
    }

    /// Emit the `SailM`/`SailME` prelude (`doc_monad_abbrev`,
    /// ~lines 1503-1522, current concurrency interface V2).
    ///
    /// The old `IO`/`ExceptT String IO` stubs disagreed with the reference
    /// backend and with the `sail-lake` golden contract: Sail functions
    /// live in `PreSailM`/`PreSailME`. The `Arch` instance and
    /// instantiation parameters live in `sail-lake`, not here. Use
    /// [`Self::print_prelude_v1`] for models on concurrency interface V1.
    pub fn print_prelude<W: fmt::Write>(&self, out: &mut W) -> Result<(), Error> {
        self.print_prelude_v2(out)
    }

    /// V2 prelude (`PreSailM exception` / `PreSailME exception`).
    pub fn print_prelude_v2<W: fmt::Write>(&self, out: &mut W) -> Result<(), Error> {
        out.write_str(
            "-- Sail monad prelude (concurrency interface V2; see doc_monad_abbrev).\n\
             -- Pure Sail functions are plain Lean values.\n\
             -- Stateful Sail functions live in `SailM` (do-notation, `readReg`/`writeReg`).\n\
             -- Early-return Sail functions run in the `ExceptM` layer\n\
             -- (`early_return x` maps to `SailME.throw x` in `SailM`, `throw x` otherwise).\n\
             abbrev exception := Unit\n\
             abbrev SailM := PreSailM exception\n\
             abbrev SailME := PreSailME exception\n",
        )?;
        Ok(())
    }

    /// V1 prelude (`PreSailM RegisterType trivialChoiceSource exception`).
    pub fn print_prelude_v1<W: fmt::Write>(&self, out: &mut W) -> Result<(), Error> {
        out.write_str(
            "-- Sail monad prelude (concurrency interface V1; see doc_monad_abbrev).\n\
             abbrev exception := Unit\n\
             abbrev SailM := PreSailM RegisterType trivialChoiceSource exception\n\
             abbrev SailME := PreSailME RegisterType trivialChoiceSource exception\n",
        )?;
        Ok(())
    }
}

/// Lean-identifier sanitizer: port of OCaml `fix_id`
/// (`pretty_print_lean.ml#98-107`), extended so output always parses.
///
/// OCaml rules kept verbatim: `_lean_wildcard` → `_`, `?` →
/// `questionMark`, `main` → `sail_main`, `#` → `_`, and
/// `rec|def|at|alias|break|meta|class` gain a trailing `'`. On top, the
/// Lean keywords this crate already guarded keep their trailing `'`,
/// and empty / digit-leading names gain a leading `_`.
fn sanitize_ident(name: &str) -> String {
    if name == "_lean_wildcard" {
        return "_".to_string();
    }
    if name == "?" {
        return "questionMark".to_string();
    }
    if name == "main" {
        return "sail_main".to_string();
    }
    const KEYWORDS: &[&str] = &[
        "def", "match", "where", "with", "if", "then", "else", "for", "in", "do", "let", "mut",
        "return", "import", "open", "namespace", "section", "end", "structure", "inductive",
        "abbrev", "theorem", "lemma", "example", "sorry", "admit", "rec", "at", "alias", "break",
        "meta", "class",
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

/// Port of OCaml `fixup_match_id` (`pretty_print_lean.ml#506-510`):
/// `Some` → `some`, `None` → `none`, `early_return` → `throw` (matching
/// the Lean standard library); everything else goes through
/// `fix_id` ([`sanitize_ident`]).
///
/// Callers print match/pattern identifiers through this; definition
/// names use [`sanitize_ident`] directly (OCaml `doc_funcl_init` calls
/// plain `doc_id_ctor` for the `def` name).
#[must_use]
pub fn fixup_match_id(name: &str) -> String {
    match name {
        "Some" => "some".to_string(),
        "None" => "none".to_string(),
        "early_return" => "throw".to_string(),
        other => sanitize_ident(other),
    }
}

/// Port of OCaml `doc_kid` (`pretty_print_lean.ml#136-139`): strip a
/// leading `'` (`'n` → `n`), prefix `k_`, sanitize. (The
/// `kid_id_renames` lookup has no counterpart in this crate yet:
/// implicit-argument renames stay a documented gap.)
#[must_use]
pub fn print_kid_name(raw: &str) -> String {
    let stripped = raw.strip_prefix('\'').unwrap_or(raw);
    sanitize_ident(&format!("k_{stripped}"))
}

/// Port of OCaml `doc_big_int` (`pretty_print_lean.ml#226`): negatives
/// print parenthesised (`(-7)`), non-negatives plain.
fn print_big_int<W: fmt::Write>(out: &mut W, i: i128) -> Result<(), Error> {
    if i < 0 {
        write!(out, "({i})")?;
    } else {
        write!(out, "{i}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
