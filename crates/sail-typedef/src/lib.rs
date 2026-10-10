#![forbid(unsafe_code)]
//! Lane 30 — structs-enums-unions: port of `doc_typdef` from the OCaml
//! Lean backend (`sail/src/sail_lean_backend/pretty_print_lean.ml:1271-1342`).
//!
//! ## Mapping (OCaml arm → Lean shape → Rust entry point)
//!
//! | Sail (`TypeDefKind`) | Lean | Notes |
//! |---|---|---|
//! | `Enum` | `inductive E where \| A \| B` + `deriving` + `open E` | [`TypedefPrinter::print_typedef`]; empty enums derive only `Repr` |
//! | `Record` | `structure S (…) where` + fields + `deriving` | No `open` emit (matches OCaml) |
//! | `Variant` | `inductive O (…) where` + `\| Ctor (_ : Ty)` arms + `deriving` + `open O` | [`TypedefPrinter::print_typedef`]; empty unions skip `Inhabited` |
//! | `Abbrev` (type) | `abbrev b … := T` | `range` bodies use relevant `(k : Kind)` params, other bodies use bare kind vars |
//! | `Abbrev` (nexp) | `abbrev x … : Int := n` | |
//! | `Abbrev` (constraint) | `abbrev x … : Bool := nc` | |
//! | `Abbrev` named `fp_bits` | `-- Abbreviation fp_bits skipped` | Deliberate skip ported from the OCaml `FIXME`; see below |
//! | `Abstract` | hard [`Error::Unsupported`] with span | Never `sorry` (E2) |
//! | `Bitfield` | hard [`Error::Unsupported`] with span | Bitfields are desugared to an abbrev plus accessors *before* Lean printing (e.g. `bitfield.expected.lean` shows `abbrev cr_type := (BitVec 8)`); reaching the printer means the desugar step was skipped |
//!
//! Deriving order is significant and reproduced exactly: enums derive
//! `BEq, Inhabited, Repr` (non-empty) or `BEq, Repr` (empty); records derive
//! `BEq, Inhabited, Repr`; non-empty variants derive `Inhabited, BEq, Repr`
//! and empty ones `BEq, Repr`. `BEq` is omitted for types listed in
//! [`Options::non_beq_types`] (mirrors the OCaml `-lean-non-beq` flag behind
//! `non_beq_types` in `pretty_print_lean.ml:38`).
//!
//! `open <Type>` is emitted inline for enums and variants, and additionally
//! reported via [`Printed::opened`] so drivers can track the `opens` set
//! (`pretty_print_lean.ml:42,1280,1332`).
//!
//! ## `fp_bits` skip
//!
//! The OCaml backend replaces `type fp_bits = …` with the comment
//! `-- Abbreviation fp_bits skipped (* FIXME *)` because the Sail
//! floating-point bitvector type has no direct Lean counterpart yet. This
//! crate ports that skip verbatim (comment text included) instead of
//! erroring, so output stays line-compatible with the reference backend.
//!
//! ## `Nat` vs `Int` binders
//!
//! The OCaml backend prints `K_int` binders as `Nat` when its typechecker can
//! prove the variable non-negative (`provably_nneg`), and as `Int` otherwise
//! — including in the `/-- Type quantifiers: … -/` comment, which is rendered
//! *without* the new binders in scope and therefore always shows `Int`.
//! This crate has no solver, so it approximates: a `K_int` kid prints as
//! `Nat` in binder position iff the same quantifier list carries a constraint
//! implying `kid ≥ 0` (`Ge`/`Gt`/`Le`/`Lt` against a constant, through
//! top-level `And`). The quantifier comment always prints `Int`, matching the
//! reference output (e.g. `struct.expected.lean` shows `k_n : Int` in the
//! comment but `(k_n : Nat)` in the binders).
//!
//! ## Layout
//!
//! The OCaml backend uses the `PPrint` combinator library (soft breaks that
//! wrap long quantifier comments and parameter lists). This crate emits the
//! same tokens with single-line layout and two-space indents, which matches
//! the reference files byte-for-byte for the common cases (`enum.sail`,
//! `option.sail` prelude block, `My_struct`) and modulo line-wrapping for
//! wide parameter lists. Differential tests normalize wrapping whitespace.
//!
//! Inputs are [`sail_ast`](sail_ast) nodes (`TypeDef`/`Typ`/`NExp`/…);
//! identifier and kind-variable spellings are resolved through
//! [`sail_ast::Interner`] (kids arrive with a leading `'`, e.g. `'n`, and
//! print as `k_n` per OCaml `doc_kid`).

use sail_ast::{
    ConstraintKind, Interner, Kind, NExp, NExpKind, QuantItem, Span, Symbol, Typ, TypArg,
    TypArgKind, TypKind, TypeDef, TypeDefKind,
};
use std::collections::BTreeSet;
use std::fmt;

/// Printer failure. There is deliberately **no** silent fallback:
/// anything outside the supported subset is a hard error carrying a span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Construct is not translatable; carries the source span of the node.
    Unsupported { msg: String, span: Span },
    /// An interned name/kid could not be resolved through the [`Interner`].
    UnknownSymbol { symbol: Symbol, span: Span },
    /// Underlying sink failed.
    Fmt(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { msg, span } => {
                write!(
                    f,
                    "cannot translate typedef to Lean at file {} [{}, {}): {msg}",
                    span.file.0, span.start, span.end
                )
            }
            Self::UnknownSymbol { symbol, span } => {
                write!(
                    f,
                    "cannot resolve interned name {} at file {} [{}, {}]",
                    symbol.0, span.file.0, span.start, span.end
                )
            }
            Self::Fmt(e) => write!(f, "typedef printer IO error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<fmt::Error> for Error {
    fn from(_: fmt::Error) -> Self {
        Self::Fmt("fmt::Write failed".to_string())
    }
}

/// Printer configuration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    /// Type names that must not derive `BEq` (mirrors the OCaml
    /// `-lean-non-beq` flag feeding `non_beq_types`).
    pub non_beq_types: BTreeSet<String>,
}

/// Outcome of printing one typedef: the `open <Type>` name when the OCaml
/// backend records one in its `opens` set (enums and variants only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printed {
    /// Name emitted in an `open …` line, if any.
    pub opened: Option<String>,
}

/// Streaming `doc_typdef` port. Cheap to clone, `Send + Sync` when `Options`
/// is (it is: `BTreeSet<String>`).
#[derive(Debug, Clone)]
pub struct TypedefPrinter {
    opts: Options,
}

impl TypedefPrinter {
    #[must_use]
    pub fn new(opts: Options) -> Self {
        Self { opts }
    }

    #[must_use]
    pub fn with_default() -> Self {
        Self::new(Options::default())
    }

    #[must_use]
    pub fn options(&self) -> &Options {
        &self.opts
    }

    /// Print one Sail type definition, streaming into `out`.
    pub fn print_typedef(
        &self,
        out: &mut impl fmt::Write,
        td: &TypeDef,
        ix: &impl Interner,
    ) -> Result<Printed, Error> {
        let cx = Cx {
            opts: &self.opts,
            ix,
        };
        cx.print_typedef(out, td)
    }

    /// Render one typedef to a fresh `String`.
    pub fn render_typedef(
        &self,
        td: &TypeDef,
        ix: &impl Interner,
    ) -> Result<(String, Printed), Error> {
        let mut buf = String::new();
        let printed = self.print_typedef(&mut buf, td, ix)?;
        Ok((buf, printed))
    }
}

// ---------------------------------------------------------------------------
// Internal rendering context
// ---------------------------------------------------------------------------

struct Cx<'a, I: Interner + ?Sized> {
    opts: &'a Options,
    ix: &'a I,
}

impl<I: Interner + ?Sized> Cx<'_, I> {
    // -- names -----------------------------------------------------------

    fn resolve(&self, symbol: Symbol, span: Span) -> Result<String, Error> {
        match self.ix.resolve(symbol) {
            Some(text) => Ok(text.to_string()),
            None => Err(Error::UnknownSymbol { symbol, span }),
        }
    }

    fn typedef_name(&self, symbol: Symbol, span: Span, what: &str) -> Result<String, Error> {
        let raw = self.resolve(symbol, span)?;
        if raw.is_empty() {
            return Err(Error::Unsupported {
                msg: format!("anonymous {what} is not translatable"),
                span,
            });
        }
        Ok(fix_id(&raw))
    }

    fn field_name(&self, symbol: Symbol, span: Span) -> Result<String, Error> {
        self.typedef_name(symbol, span, "field")
    }

    fn kid_name(&self, kid: Symbol, span: Span) -> Result<String, Error> {
        let raw = self.resolve(kid, span)?;
        let stripped = raw.strip_prefix('\'').unwrap_or(&raw);
        if stripped.is_empty() {
            return Err(Error::Unsupported {
                msg: "anonymous kind variable is not translatable".to_string(),
                span,
            });
        }
        Ok(fix_id(&format!("k_{stripped}")))
    }

    // -- top-level dispatch ----------------------------------------------

    fn print_typedef(&self, out: &mut impl fmt::Write, td: &TypeDef) -> Result<Printed, Error> {
        match &td.kind {
            TypeDefKind::Enum { name, members, .. } => {
                let id = self.typedef_name(name.symbol, name.span, "enum")?;
                let mut ctors = Vec::with_capacity(members.len());
                for member in members {
                    ctors.push(self.typedef_name(member.symbol, member.span, "enum member")?);
                }
                write!(out, "inductive {id} where")?;
                for ctor in &ctors {
                    write!(out, " | {ctor}")?;
                }
                let derivers = enum_derivers(&id, ctors.is_empty(), self.opts);
                writeln!(out, "\n  deriving {}", derivers.join(", "))?;
                writeln!(out, "  open {id}")?;
                Ok(Printed { opened: Some(id) })
            }
            TypeDefKind::Record {
                name,
                quant,
                fields,
                ..
            } => {
                let id = self.typedef_name(name.symbol, name.span, "struct")?;
                self.print_quant_comment(out, quant, td.span)?;
                write!(out, "structure {id}")?;
                self.print_params(out, quant, td.span)?;
                out.write_str(" where\n")?;
                for (field, ty) in fields {
                    let fname = self.field_name(field.symbol, field.span)?;
                    write!(out, "  {fname} : ")?;
                    self.print_typ(out, ty)?;
                    out.write_char('\n')?;
                }
                let derivers = record_derivers(&id, self.opts);
                writeln!(out, "  deriving {}", derivers.join(", "))?;
                Ok(Printed { opened: None })
            }
            TypeDefKind::Variant {
                name, quant, ctors, ..
            } => {
                let id = self.typedef_name(name.symbol, name.span, "union")?;
                self.print_quant_comment(out, quant, td.span)?;
                write!(out, "inductive {id}")?;
                self.print_params(out, quant, td.span)?;
                out.write_str(" where\n")?;
                for ctor in ctors {
                    let cname =
                        self.typedef_name(ctor.ctor.symbol, ctor.ctor.span, "union constructor")?;
                    write!(out, "  | {cname} (_ : ")?;
                    self.print_typ(out, &ctor.typ)?;
                    out.write_str(")\n")?;
                }
                let derivers = variant_derivers(&id, ctors.is_empty(), self.opts);
                writeln!(out, "  deriving {}", derivers.join(", "))?;
                writeln!(out, "  open {id}")?;
                Ok(Printed { opened: Some(id) })
            }
            TypeDefKind::Abbrev { name, quant, arg } => {
                let id = self.typedef_name(name.symbol, name.span, "abbrev")?;
                match &arg.kind {
                    // `range` bodies keep relevant `(k : Kind)` binders.
                    TypArgKind::Typ(body) if is_range_app(self.ix, body) => {
                        write!(out, "abbrev {id}")?;
                        self.print_params(out, quant, arg.span)?;
                        out.write_str(" := ")?;
                        self.print_typ(out, body)?;
                        out.write_char('\n')?;
                    }
                    // Documented `FIXME` skip, ported verbatim.
                    TypArgKind::Typ(body) if id == "fp_bits" => {
                        out.write_str("-- Abbreviation fp_bits skipped\n")?;
                        let _ = body;
                    }
                    TypArgKind::Typ(body) => {
                        write!(out, "abbrev {id}")?;
                        self.print_vars_only(out, quant, arg.span)?;
                        out.write_str(" := ")?;
                        self.print_typ(out, body)?;
                        out.write_char('\n')?;
                    }
                    TypArgKind::NExp(body) => {
                        write!(out, "abbrev {id}")?;
                        self.print_params(out, quant, arg.span)?;
                        out.write_str(" : Int := ")?;
                        Self::print_nexp_static(self, out, body)?;
                        out.write_char('\n')?;
                    }
                    TypArgKind::Bool(body) => {
                        write!(out, "abbrev {id}")?;
                        self.print_params(out, quant, arg.span)?;
                        out.write_str(" : Bool := ")?;
                        self.print_constraint(out, body)?;
                        out.write_char('\n')?;
                    }
                }
                Ok(Printed { opened: None })
            }
            TypeDefKind::Abstract { name, .. } => {
                let id = self.resolve(name.symbol, name.span)?;
                Err(Error::Unsupported {
                    msg: format!(
                        "type `{id}` is declared abstract (TD_abstract) and has no Lean representation; \
                         provide an extern mapping or remove it (hard error by design, never a silent fallback)"
                    ),
                    span: td.span,
                })
            }
            TypeDefKind::Bitfield { name, .. } => {
                let id = self.resolve(name.symbol, name.span)?;
                Err(Error::Unsupported {
                    msg: format!(
                        "bitfield `{id}` (TD_bitfield) must be desugared to an abbrev plus accessor \
                         functions before Lean printing (hard error by design, never a silent fallback)"
                    ),
                    span: td.span,
                })
            }
        }
    }

    // -- quantifiers ------------------------------------------------------

    /// `/-- Type quantifiers: … -/` comment; `K_int` always prints as `Int`
    /// here (binders are out of scope, matching the reference backend).
    fn print_quant_comment(
        &self,
        out: &mut impl fmt::Write,
        quant: &[QuantItem],
        span: Span,
    ) -> Result<(), Error> {
        if quant.is_empty() {
            return Ok(());
        }
        let mut items = Vec::with_capacity(quant.len());
        for item in quant {
            match item {
                QuantItem::Id(kopt) => {
                    let kid = self.kid_name(kopt.kid, span)?;
                    items.push(format!("{kid} : {}", comment_kind(kopt.kind)));
                }
                QuantItem::Constraint(nc) => {
                    let mut buf = String::new();
                    self.print_constraint(&mut buf, nc)?;
                    items.push(buf);
                }
            }
        }
        writeln!(out, "/-- Type quantifiers: {} -/", items.join(", "))?;
        Ok(())
    }

    /// Relevant `(k : Kind)` binders; constraints are dropped.
    fn print_params(
        &self,
        out: &mut impl fmt::Write,
        quant: &[QuantItem],
        span: Span,
    ) -> Result<(), Error> {
        for item in quant {
            if let QuantItem::Id(kopt) = item {
                let kid = self.kid_name(kopt.kid, span)?;
                let kind = binder_kind(quant, kopt);
                write!(out, " ({kid} : {kind})")?;
            }
        }
        Ok(())
    }

    /// Bare kind variables for type-abbrevs (`doc_typ_quant_only_vars`).
    fn print_vars_only(
        &self,
        out: &mut impl fmt::Write,
        quant: &[QuantItem],
        span: Span,
    ) -> Result<(), Error> {
        for item in quant {
            if let QuantItem::Id(kopt) = item {
                let kid = self.kid_name(kopt.kid, span)?;
                write!(out, " {kid}")?;
            }
        }
        Ok(())
    }

    // -- types --------------------------------------------------------------

    fn print_typ(&self, out: &mut impl fmt::Write, ty: &Typ) -> Result<(), Error> {
        match &ty.kind {
            TypKind::Unknown => Err(Error::Unsupported {
                msg: "type hole (Typ_internal_unknown) is not translatable".to_string(),
                span: ty.span,
            }),
            TypKind::Id(name) => {
                let raw = self.resolve(name.symbol, name.span)?;
                match raw.as_str() {
                    "unit" => out.write_str("Unit")?,
                    "int" => out.write_str("Int")?,
                    "string" => out.write_str("String")?,
                    "bool" | "atom_bool" => out.write_str("Bool")?,
                    "bit" => out.write_str("(BitVec 1)")?,
                    "nat" => out.write_str("Nat")?,
                    _ => out.write_str(&fix_id(&raw))?,
                }
                Ok(())
            }
            TypKind::Var(kid) => {
                let name = self.kid_name(*kid, ty.span)?;
                out.write_str(&name)?;
                Ok(())
            }
            TypKind::Fn { .. } => Err(Error::Unsupported {
                msg: "function type (Typ_fn) is not translatable in typedef position".to_string(),
                span: ty.span,
            }),
            TypKind::Bidir { .. } => Err(Error::Unsupported {
                msg: "bidirectional type (Typ_bidir) is not translatable in typedef position"
                    .to_string(),
                span: ty.span,
            }),
            TypKind::Tuple(items) => {
                if items.is_empty() {
                    out.write_str("()")?;
                    return Ok(());
                }
                out.write_char('(')?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.write_str(" × ")?;
                    }
                    self.print_typ(out, item)?;
                }
                out.write_char(')')?;
                Ok(())
            }
            TypKind::App { name, args } => {
                let head = self.resolve(name.symbol, name.span)?;
                match head.as_str() {
                    "vector" => match args.as_slice() {
                        [m, e] if is_nexp_arg(m) && is_typ_arg(e) => {
                            out.write_str("(Vector ")?;
                            self.print_typ(out, expect_typ(e))?;
                            out.write_char(' ')?;
                            Self::print_nexp_static(self, out, expect_nexp(m))?;
                            out.write_char(')')?;
                            Ok(())
                        }
                        _ => self.print_generic_app(out, ty.span, &head, args),
                    },
                    "bitvector" | "bits" => match args.as_slice() {
                        [m] if is_nexp_arg(m) => {
                            out.write_str("(BitVec ")?;
                            Self::print_nexp_static(self, out, expect_nexp(m))?;
                            out.write_char(')')?;
                            Ok(())
                        }
                        _ => self.print_generic_app(out, ty.span, &head, args),
                    },
                    // `atom(x)`: `Nat` when provably non-negative, else
                    // `Int`. `int('n)` is accepted with the same rule: the
                    // Sail frontend normalises it before Lean printing, and
                    // `struct.expected.lean` shows `size : int('n)` as `Int`.
                    "atom" | "int" => match args.as_slice() {
                        [m] if is_nexp_arg(m) => {
                            out.write_str(if nexp_is_nat_const(expect_nexp(m)) {
                                "Nat"
                            } else {
                                "Int"
                            })?;
                            Ok(())
                        }
                        _ => self.print_generic_app(out, ty.span, &head, args),
                    },
                    // OCaml joins register args with spaceless `comma`
                    // (`(RegisterRef a,b)`), not `comma_sp`.
                    "register" => {
                        let mut rendered = Vec::with_capacity(args.len());
                        for arg in args {
                            let mut buf = String::new();
                            self.print_typ_arg_all(&mut buf, arg)?;
                            rendered.push(buf);
                        }
                        write!(out, "(RegisterRef {})", rendered.join(","))?;
                        Ok(())
                    }
                    "implicit" => match args.as_slice() {
                        [m] if is_nexp_arg(m) => {
                            out.write_char('_')?;
                            Ok(())
                        }
                        _ => self.print_generic_app(out, ty.span, &head, args),
                    },
                    "option" => match args.as_slice() {
                        [t] if is_typ_arg(t) => {
                            out.write_str("(Option ")?;
                            self.print_typ(out, expect_typ(t))?;
                            out.write_char(')')?;
                            Ok(())
                        }
                        _ => self.print_generic_app(out, ty.span, &head, args),
                    },
                    "list" => {
                        out.write_str("(List")?;
                        for arg in args {
                            if let TypArgKind::Bool(_) = &arg.kind {
                                continue;
                            }
                            out.write_char(' ')?;
                            self.print_typ_arg_relevant(out, arg)?;
                        }
                        out.write_char(')')?;
                        Ok(())
                    }
                    "range" => match args.as_slice() {
                        [lo, _] if is_nexp_arg(lo) => {
                            out.write_str(if nexp_is_nat_const(expect_nexp(lo)) {
                                "Nat"
                            } else {
                                "Int"
                            })?;
                            Ok(())
                        }
                        _ => self.print_generic_app(out, ty.span, &head, args),
                    },
                    "result" => match args.as_slice() {
                        [a, b] if is_typ_arg(a) && is_typ_arg(b) => {
                            out.write_str("(Result ")?;
                            self.print_typ(out, expect_typ(a))?;
                            out.write_char(' ')?;
                            self.print_typ(out, expect_typ(b))?;
                            out.write_char(')')?;
                            Ok(())
                        }
                        _ => self.print_generic_app(out, ty.span, &head, args),
                    },
                    _ => self.print_generic_app(out, ty.span, &head, args),
                }
            }
            TypKind::Exist { typ, .. } => self.print_typ(out, typ),
        }
    }

    /// Generic named application with only-relevant args
    /// (bool constraints dropped, as in OCaml `` `Only_relevant ``).
    fn print_generic_app(
        &self,
        out: &mut impl fmt::Write,
        span: Span,
        head: &str,
        args: &[TypArg],
    ) -> Result<(), Error> {
        let mut rendered: Vec<String> = Vec::new();
        for arg in args {
            if let TypArgKind::Bool(_) = &arg.kind {
                continue;
            }
            let mut buf = String::new();
            self.print_typ_arg_relevant(&mut buf, arg)?;
            rendered.push(buf);
        }
        if rendered.is_empty() {
            if head.is_empty() {
                return Err(Error::Unsupported {
                    msg: "anonymous type application is not translatable".to_string(),
                    span,
                });
            }
            out.write_str(&fix_id(head))?;
        } else {
            write!(out, "({} {})", fix_id(head), rendered.join(" "))?;
        }
        Ok(())
    }

    fn print_typ_arg_relevant(&self, out: &mut impl fmt::Write, arg: &TypArg) -> Result<(), Error> {
        match &arg.kind {
            TypArgKind::Typ(t) => self.print_typ(out, t),
            TypArgKind::NExp(n) => Self::print_nexp_static(self, out, n),
            TypArgKind::Bool(_) => Err(Error::Unsupported {
                msg: "bool constraint argument is dropped in relevant-only position".to_string(),
                span: arg.span,
            }),
        }
    }

    fn print_typ_arg_all(&self, out: &mut impl fmt::Write, arg: &TypArg) -> Result<(), Error> {
        match &arg.kind {
            TypArgKind::Typ(t) => self.print_typ(out, t),
            TypArgKind::NExp(n) => Self::print_nexp_static(self, out, n),
            TypArgKind::Bool(nc) => {
                out.write_char('(')?;
                self.print_constraint(out, nc)?;
                out.write_char(')')?;
                Ok(())
            }
        }
    }

    // -- size expressions -----------------------------------------------------

    fn print_nexp_static(&self, out: &mut impl fmt::Write, n: &NExp) -> Result<(), Error> {
        // Top level mirrors OCaml `doc_nexp`: only constants, variables and
        // plain identifiers print bare; every compound expression is
        // parenthesised (`(BitVec (8 * k_n))`, never `(BitVec 8 * k_n)`).
        match &n.kind {
            NExpKind::Constant(_) | NExpKind::Var(_) | NExpKind::Id(_) => {
                self.print_nexp_atom(out, n)
            }
            _ => {
                out.write_char('(')?;
                Self::print_nexp_sum(self, out, n)?;
                out.write_char(')')?;
                Ok(())
            }
        }
    }

    fn print_nexp_sum(&self, out: &mut impl fmt::Write, n: &NExp) -> Result<(), Error> {
        match &n.kind {
            NExpKind::Sum(a, b) => {
                self.print_nexp_sum(out, a)?;
                out.write_str(" + ")?;
                self.print_nexp_product(out, b)?;
                Ok(())
            }
            NExpKind::Minus(a, b) => {
                self.print_nexp_sum(out, a)?;
                out.write_str(" - ")?;
                self.print_nexp_product(out, b)?;
                Ok(())
            }
            _ => self.print_nexp_product(out, n),
        }
    }

    fn print_nexp_product(&self, out: &mut impl fmt::Write, n: &NExp) -> Result<(), Error> {
        match &n.kind {
            NExpKind::Times(a, b) => {
                self.print_nexp_product(out, a)?;
                out.write_str(" * ")?;
                self.print_nexp_unary(out, b)?;
                Ok(())
            }
            _ => self.print_nexp_unary(out, n),
        }
    }

    fn print_nexp_unary(&self, out: &mut impl fmt::Write, n: &NExp) -> Result<(), Error> {
        match &n.kind {
            NExpKind::Neg(inner) => {
                out.write_str("(- ")?;
                self.print_nexp_unary(out, inner)?;
                out.write_char(')')?;
                Ok(())
            }
            _ => self.print_nexp_pow(out, n),
        }
    }

    fn print_nexp_pow(&self, out: &mut impl fmt::Write, n: &NExp) -> Result<(), Error> {
        match &n.kind {
            NExpKind::Exp(e) => {
                out.write_str("2 ^ ")?;
                self.print_nexp_pow(out, e)?;
                Ok(())
            }
            _ => self.print_nexp_app(out, n),
        }
    }

    fn print_nexp_app(&self, out: &mut impl fmt::Write, n: &NExp) -> Result<(), Error> {
        match &n.kind {
            NExpKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                out.write_str("if (")?;
                self.print_constraint(out, cond)?;
                out.write_str(" : Bool) then ")?;
                self.print_nexp_atom(out, then_branch)?;
                out.write_str(" else ")?;
                self.print_nexp_atom(out, else_branch)?;
                Ok(())
            }
            NExpKind::App { func, args } => {
                let head = self.resolve(func.symbol, func.span)?;
                match (head.as_str(), args.as_slice()) {
                    ("div", [a, b]) => {
                        self.print_nexp_atom(out, a)?;
                        out.write_str(" / ")?;
                        self.print_nexp_atom(out, b)?;
                        Ok(())
                    }
                    ("mod", [a, b]) => {
                        self.print_nexp_atom(out, a)?;
                        out.write_str(" % ")?;
                        self.print_nexp_atom(out, b)?;
                        Ok(())
                    }
                    ("abs", [a]) => {
                        self.print_nexp_atom(out, a)?;
                        out.write_str(".natAbs")?;
                        Ok(())
                    }
                    _ => Err(Error::Unsupported {
                        msg: format!("size-expression application `{head}` is not translatable"),
                        span: n.span,
                    }),
                }
            }
            _ => self.print_nexp_atom(out, n),
        }
    }

    fn print_nexp_atom(&self, out: &mut impl fmt::Write, n: &NExp) -> Result<(), Error> {
        match &n.kind {
            NExpKind::Constant(text) => {
                let digits = text.strip_prefix('-').unwrap_or(text);
                if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                    if text.starts_with('-') {
                        write!(out, "({text})")?;
                    } else {
                        out.write_str(text)?;
                    }
                    Ok(())
                } else {
                    Err(Error::Unsupported {
                        msg: format!("numeric constant `{text}` is not translatable"),
                        span: n.span,
                    })
                }
            }
            NExpKind::Var(kid) => {
                let name = self.kid_name(*kid, n.span)?;
                out.write_str(&name)?;
                Ok(())
            }
            NExpKind::Id(name) => {
                let id = self.typedef_name(name.symbol, name.span, "size variable")?;
                out.write_str(&id)?;
                Ok(())
            }
            NExpKind::Sum(..)
            | NExpKind::Minus(..)
            | NExpKind::Times(..)
            | NExpKind::Neg(..)
            | NExpKind::Exp(..)
            | NExpKind::If { .. }
            | NExpKind::App { .. } => {
                out.write_char('(')?;
                self.print_nexp_sum(out, n)?;
                out.write_char(')')?;
                Ok(())
            }
        }
    }

    // -- constraints ------------------------------------------------------------

    fn print_constraint(
        &self,
        out: &mut impl fmt::Write,
        nc: &sail_ast::Constraint,
    ) -> Result<(), Error> {
        match &nc.kind {
            ConstraintKind::And(a, b) => {
                self.print_constraint(out, a)?;
                out.write_str(" ∧ ")?;
                self.print_constraint(out, b)?;
                Ok(())
            }
            ConstraintKind::Or(a, b) => {
                self.print_constraint(out, a)?;
                out.write_str(" ∨ ")?;
                self.print_constraint(out, b)?;
                Ok(())
            }
            ConstraintKind::Not(x) => {
                out.write_str("¬")?;
                self.print_constraint(out, x)?;
                Ok(())
            }
            ConstraintKind::Equal(a, b) => {
                self.print_typ_arg_all(out, a)?;
                out.write_str(" = ")?;
                self.print_typ_arg_all(out, b)?;
                Ok(())
            }
            ConstraintKind::NotEqual(a, b) => {
                self.print_typ_arg_all(out, a)?;
                out.write_str(" ≠ ")?;
                self.print_typ_arg_all(out, b)?;
                Ok(())
            }
            ConstraintKind::Ge(a, b) => {
                Self::print_nexp_static(self, out, a)?;
                out.write_str(" ≥ ")?;
                Self::print_nexp_static(self, out, b)?;
                Ok(())
            }
            ConstraintKind::Gt(a, b) => {
                Self::print_nexp_static(self, out, a)?;
                out.write_str(" > ")?;
                Self::print_nexp_static(self, out, b)?;
                Ok(())
            }
            ConstraintKind::Le(a, b) => {
                Self::print_nexp_static(self, out, a)?;
                out.write_str(" ≤ ")?;
                Self::print_nexp_static(self, out, b)?;
                Ok(())
            }
            ConstraintKind::Lt(a, b) => {
                Self::print_nexp_static(self, out, a)?;
                out.write_str(" < ")?;
                Self::print_nexp_static(self, out, b)?;
                Ok(())
            }
            ConstraintKind::Set { nexp, members } => {
                out.write_str("List.elem ")?;
                Self::print_nexp_static(self, out, nexp)?;
                write!(out, " [{}]", members.join(", "))?;
                Ok(())
            }
            ConstraintKind::App { func, args } => {
                let head = self.typedef_name(func.symbol, func.span, "constraint")?;
                out.write_char('(')?;
                out.write_str(&head)?;
                for arg in args {
                    out.write_char(' ')?;
                    self.print_typ_arg_all(out, arg)?;
                }
                out.write_char(')')?;
                Ok(())
            }
            ConstraintKind::Id(name) => {
                let id = self.typedef_name(name.symbol, name.span, "constraint")?;
                out.write_str(&id)?;
                Ok(())
            }
            ConstraintKind::Var(kid) => {
                let name = self.kid_name(*kid, nc.span)?;
                out.write_str(&name)?;
                Ok(())
            }
            ConstraintKind::True => {
                out.write_str("true")?;
                Ok(())
            }
            ConstraintKind::False => {
                out.write_str("false")?;
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Port of OCaml `fix_id` for the identifier positions used by `doc_typdef`.
/// `main` becomes `sail_main`, `#` becomes `_`, a small keyword list gains a
/// trailing `'`. Unlike the OCaml version this also guards empty and
/// digit-leading names so output always parses.
fn fix_id(name: &str) -> String {
    if name == "_lean_wildcard" {
        return "_".to_string();
    }
    if name == "?" {
        return "questionMark".to_string();
    }
    let mut out = name.replace('#', "_");
    match out.as_str() {
        "rec" | "def" | "at" | "alias" | "break" | "meta" | "class" => out.push('\''),
        "main" => out = "sail_main".to_string(),
        _ => {}
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_numeric()) {
        out.insert(0, '_');
    }
    out
}

/// `K_int` binder names: `Nat` when the quantifier list proves non-negativity
/// (heuristic stand-in for `provably_nneg`), else `Int`.
fn binder_kind(quant: &[QuantItem], kopt: &sail_ast::KOpt) -> &'static str {
    match kopt.kind {
        Kind::Type => "Type",
        Kind::Bool => "Bool",
        Kind::Int => {
            if quant
                .iter()
                .filter_map(|item| match item {
                    QuantItem::Constraint(nc) => Some(&nc.kind),
                    QuantItem::Id(_) => None,
                })
                .any(|nc| implies_nneg(nc, kopt.kid))
            {
                "Nat"
            } else {
                "Int"
            }
        }
    }
}

/// Quantifier comments always use `Int` for `K_int` (binders out of scope).
fn comment_kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Type => "Type",
        Kind::Bool => "Bool",
        Kind::Int => "Int",
    }
}

/// Heuristic stand-in for the typechecker's `provably_nneg`: does this
/// constraint imply `kid ≥ 0`?
fn implies_nneg(nc: &ConstraintKind, kid: Symbol) -> bool {
    match nc {
        // `k ≥ c` proves `k ≥ 0` iff `c ≥ 0` (and mirrored `c ≤ k`).
        ConstraintKind::Ge(a, b) => nexp_is_kid(a, kid) && nexp_const_ge(b, 0),
        ConstraintKind::Gt(a, b) => nexp_is_kid(a, kid) && nexp_const_ge(b, -1),
        ConstraintKind::Le(a, b) => nexp_is_kid(b, kid) && nexp_const_ge(a, 0),
        ConstraintKind::Lt(a, b) => nexp_is_kid(b, kid) && nexp_const_ge(a, -1),
        ConstraintKind::And(x, y) => implies_nneg(&x.kind, kid) || implies_nneg(&y.kind, kid),
        _ => false,
    }
}

fn nexp_is_kid(n: &NExp, kid: Symbol) -> bool {
    matches!(&n.kind, NExpKind::Var(k) if *k == kid)
}

fn nexp_const(n: &NExp) -> Option<i128> {
    match &n.kind {
        NExpKind::Constant(text) => text.parse().ok(),
        _ => None,
    }
}

fn nexp_const_ge(n: &NExp, bound: i128) -> bool {
    nexp_const(n).is_some_and(|c| c >= bound)
}

/// `atom(x)` / `range(lo, _)` print as `Nat` for constant `x`/`lo ≥ 0`.
fn nexp_is_nat_const(n: &NExp) -> bool {
    nexp_const_ge(n, 0)
}

fn is_typ_arg(arg: &TypArg) -> bool {
    matches!(&arg.kind, TypArgKind::Typ(_))
}

fn is_nexp_arg(arg: &TypArg) -> bool {
    matches!(&arg.kind, TypArgKind::NExp(_))
}

fn expect_typ(arg: &TypArg) -> &Typ {
    match &arg.kind {
        TypArgKind::Typ(t) => t,
        _ => unreachable!("caller checked is_typ_arg"),
    }
}

fn expect_nexp(arg: &TypArg) -> &NExp {
    match &arg.kind {
        TypArgKind::NExp(n) => n,
        _ => unreachable!("caller checked is_nexp_arg"),
    }
}

/// Abbrev bodies that are `range(lo, hi)` applications take relevant
/// `(k : Kind)` binders instead of bare kind variables.
fn is_range_app<I: Interner + ?Sized>(ix: &I, body: &Typ) -> bool {
    if let TypKind::App { name, .. } = &body.kind {
        return ix.resolve(name.symbol).is_some_and(|text| text == "range");
    }
    false
}

/// Enum deriving order: `BEq` first (unless opted out), then `Inhabited`
/// for non-empty enums, then `Repr`.
fn enum_derivers(id: &str, empty: bool, opts: &Options) -> Vec<&'static str> {
    let mut derivers: Vec<&'static str> = if empty {
        vec!["Repr"]
    } else {
        vec!["Inhabited", "Repr"]
    };
    if !opts.non_beq_types.contains(id) {
        derivers.insert(0, "BEq");
    }
    derivers
}

/// Records always derive `Inhabited`; `BEq` unless opted out.
fn record_derivers(id: &str, opts: &Options) -> Vec<&'static str> {
    let mut derivers = vec!["Inhabited", "Repr"];
    if !opts.non_beq_types.contains(id) {
        derivers.insert(0, "BEq");
    }
    derivers
}

/// Variant deriving order: `Inhabited` first for non-empty unions, then
/// `BEq` (unless opted out), then `Repr`.
fn variant_derivers(id: &str, empty: bool, opts: &Options) -> Vec<&'static str> {
    let mut derivers = vec!["Repr"];
    if !opts.non_beq_types.contains(id) {
        derivers.insert(0, "BEq");
    }
    if !empty {
        derivers.insert(0, "Inhabited");
    }
    derivers
}

#[cfg(test)]
mod tests;
