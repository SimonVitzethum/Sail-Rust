#![forbid(unsafe_code)]
//! Real function-body printer: canonical `sail_ast::Exp` onto Lean.
//!
//! Port of the translatable `doc_exp`/`doc_match_clause`/`wrap_exp` cases
//! (`sail/src/sail_lean_backend/pretty_print_lean.ml`, ~lines 829-1095):
//! identifiers (registers read via `readReg`), literals (`width:true`),
//! `None`/`Some`/`__id`/`early_return`/plain applications, tuples, `let`
//! (with `prepend_monad` ascriptions), records, `match`/`match_bv`, register
//! writes, `if`, `.Reg`, `sailThrow`/`sailTryCatch[E]`, `assert`, lists,
//! `E_for`, and type-ascribed vector literals. Anything else is a hard
//! [`Error::Unsupported`] — never a silent `sorry`.
//!
//! Documented gaps against the OCaml reference (all fail loudly):
//! - Effect/type environments: the caller supplies register names,
//!   effectful (non-pure) function names and `lean`-extern spellings via
//!   [`ExpCtx`]. Unknown function names count as pure; operators without a
//!   `lean`-extern entry are rejected (their Lean spelling lives in the
//!   Sail `pragma lean(...)` table). Implicit-argument filtering
//!   (`doc_implicit_args`, #1188+) is not reproduced: all arguments print
//!   positionally.
//! - `E_vector` chooses its Lean shape from its *type* (`is_bitvector_typ`):
//!   only a syntactically ascribed `E_typ (bits(n)/bitvector(n), E_vector …)`
//!   (or `E_typ (vector …)`) prints; a bare `E_vector` is rejected.
//! - Loop combinators (`while#`/`until#`/`foreach#`, incl. `#t` fuel forms),
//!   `E_loop`, `E_var`, `E_block` sequencing in pure position, `E_sizeof`,
//!   `E_constraint`, `E_config`, `E_undef` and the internal
//!   value/assume forms have no `doc_exp` counterpart (OCaml `failwith`)
//!   and stay hard errors here too. `E_return` prints like the elaborated
//!   `early_return` application (superset of OCaml, same semantics).
//! - `is_unit` for the drop-the-binding `let` rule is approximated
//!   syntactically (unit literal on the right-hand side); anything else
//!   keeps its binder.
//!
//! Entry points: [`LeanPrinter::print_exp_top`] for a bare body and
//! [`LeanPrinter::render_fun_def`] / [`LeanPrinter::print_fun_def`] for a
//! complete `def` (signature frame from [`FunSig`] plus printed body).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use sail_ast::{
    Exp as AstExp, ExpKind, FExp as AstFExp, Interner, LExp as AstLExp, LExpKind, Name, Order, PExp as AstPExp,
    PExpKind,
};

use crate::{
    Error, FunSig, LeanPrinter, Typ,
    conv::typ_from_ast,
    pats::{Pat, is_match_bv, pat_from_ast},
};

/// Caller-supplied context for body printing: the environment fragments
/// `pretty_print_lean.ml` reads from its typechecker state.
///
/// The monad flags come from the enclosing [`FunSig`] (see
/// [`LeanPrinter::render_fun_def`]), never from this struct directly:
/// `registers` mirrors `Env.is_register`, `effectful_fns` mirrors
/// `not (Effects.function_is_pure …)`, and `lean_externs` mirrors
/// `Env.get_extern … "lean"` (Sail name → extern spelling such as
/// `_lean_add`, resolved further by [`lean_binop_of_extern`]).
#[derive(Debug, Clone, Default)]
pub struct ExpCtx {
    /// `Env.is_register`: these identifiers read via `readReg`.
    pub registers: BTreeSet<String>,
    /// `not (Effects.function_is_pure …)`: calls needing the monadic
    /// arrow discipline. Unknown names count as pure (documented gap:
    /// the driver must fill this set from effect analysis).
    pub effectful_fns: BTreeSet<String>,
    /// `Env.get_extern … "lean"`: Sail function name → Lean spelling
    /// (`_lean_add`, …). Needed for operator applications in particular.
    pub lean_externs: BTreeMap<String, String>,
}

impl ExpCtx {
    /// Context without any environment knowledge (pure bodies only).
    #[must_use]
    pub fn pure() -> Self {
        Self::default()
    }
}

/// Lean spelling of an extern id (`op_of_id`,
/// `pretty_print_lean.ml`, `let op_of_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternOp {
    Binop(&'static str),
    Unop(&'static str),
    NotOp,
}

/// Port of OCaml `op_of_id`: only the `Some(extern)` spellings map;
/// anything else (including `None`) is a plain call.
fn lean_binop_of_extern(extern_spelling: Option<&str>) -> ExternOp {
    match extern_spelling {
        Some("_lean_and") => ExternOp::Binop("&&"),
        Some("_lean_or") => ExternOp::Binop("||"),
        Some("_lean_beq") => ExternOp::Binop("=="),
        Some("_lean_bne") => ExternOp::Binop("!="),
        Some("_lean_not") => ExternOp::Unop("!"),
        Some("_lean_add") => ExternOp::Binop("+"),
        Some("_lean_addi") => ExternOp::Binop("+i"),
        Some("_lean_sub") => ExternOp::Binop("-"),
        Some("_lean_subi") => ExternOp::Binop("-i"),
        Some("_lean_mul") => ExternOp::Binop("*"),
        Some("_lean_muli") => ExternOp::Binop("*i"),
        Some("_lean_div") => ExternOp::Binop("/"),
        Some("_lean_app") => ExternOp::Binop("+++"),
        Some("_lean_bvand") => ExternOp::Binop("&&&"),
        Some("_lean_bvor") => ExternOp::Binop("|||"),
        Some("_lean_bvxor") => ExternOp::Binop("^^^"),
        Some("_lean_shiftl") => ExternOp::Binop("<<<"),
        Some("_lean_shiftr") => ExternOp::Binop(">>>"),
        Some("_lean_lt") => ExternOp::Binop("<b"),
        Some("_lean_ge") => ExternOp::Binop("≥b"),
        Some("_lean_le") => ExternOp::Binop("≤b"),
        Some("_lean_gt") => ExternOp::Binop(">b"),
        Some("_lean_pow2") => ExternOp::Unop("2 ^"),
        Some("_lean_pow2i") => ExternOp::Unop("2 ^i"),
        _ => ExternOp::NotOp,
    }
}

/// Resolve an identifier occurrence through the caller's interner.
fn name_text(name: &Name, it: &impl Interner) -> Result<String, Error> {
    it.resolve(name.symbol).map(str::to_owned).ok_or_else(|| Error::Unsupported {
        msg: format!("unresolvable identifier (symbol {})", name.symbol),
        span: name.span,
    })
}

/// `is_anonymous_pat` (`pretty_print_lean.ml#~460`): wildcards, `_`-led
/// binders and literals carry no observable binding.
fn pat_is_anonymous(pat: &Pat) -> bool {
    match pat {
        Pat::Wild => true,
        Pat::Id(name) => name.starts_with('_'),
        Pat::Lit(_) => true,
        Pat::Typ { pat, .. } | Pat::Var { pat } | Pat::As { pat, .. } => pat_is_anonymous(pat),
        _ => false,
    }
}

impl LeanPrinter {
    /// Print a complete function definition: signature header plus frame
    /// plus the *printed* body (no golden strings).
    ///
    /// Monad flags come from `sig` (`is_monadic` → `in_sail_monad`,
    /// `early_return` → `in_except_monad`); `ctx` supplies the
    /// environment fragments. The body prints with
    /// `as_monadic = sig.is_monadic`, mirroring `doc_funcl_body`
    /// (`doc_exp is_monadic …`, ~line 1229).
    pub fn print_fun_def<W: fmt::Write, I: Interner>(
        &self,
        out: &mut W,
        sig: &FunSig,
        body: &AstExp,
        it: &I,
        ctx: &ExpCtx,
    ) -> Result<(), Error> {
        self.print_sig_header(out, sig)?;
        let frame = Self::function_frame_suffix(sig.is_monadic, sig.early_return, sig.has_loop);
        let mut text = String::new();
        let body_ctx = FnBodyCtx {
            in_sail_monad: sig.is_monadic,
            in_except_monad: sig.early_return,
            env: ctx,
        };
        self.print_exp_inner(&mut text, body, it, &body_ctx, sig.is_monadic)?;
        if frame.is_empty() {
            out.write_char(' ')?;
            out.write_str(&text)?;
        } else {
            out.write_char(' ')?;
            out.write_str(frame)?;
            out.write_str("\n  ")?;
            out.write_str(&text)?;
        }
        out.write_char('\n')?;
        Ok(())
    }

    /// Render a complete function definition to a fresh `String`.
    pub fn render_fun_def<I: Interner>(
        &self,
        sig: &FunSig,
        body: &AstExp,
        it: &I,
        ctx: &ExpCtx,
    ) -> Result<String, Error> {
        let mut buf = String::new();
        self.print_fun_def(&mut buf, sig, body, it, ctx)?;
        Ok(buf)
    }

    /// Print a bare body in the monad position implied by `ctx`
    /// (`in_sail_monad` selects `as_monadic`, mirroring the top-level
    /// `doc_exp is_monadic …` call).
    pub fn print_exp_top<W: fmt::Write, I: Interner>(
        &self,
        out: &mut W,
        exp: &AstExp,
        it: &I,
        ctx: &ExpCtx,
        in_sail_monad: bool,
        in_except_monad: bool,
    ) -> Result<(), Error> {
        let body_ctx = FnBodyCtx { in_sail_monad, in_except_monad, env: ctx };
        self.print_exp_inner(out, exp, it, &body_ctx, in_sail_monad)
    }

    /// `d_of_arg` (`pretty_print_lean.ml#~840-857`): wrap an argument by
    /// shape. Effectful/`has_loop` arguments become `(← do …)` (or
    /// `(do …)` when `with_arrow` is false, e.g. inside `sailTryCatch`);
    /// `let`/`if`/`match`/… stay parenthesised when pure.
    fn print_arg<W: fmt::Write, I: Interner>(
        &self,
        out: &mut W,
        arg: &AstExp,
        it: &I,
        ctx: &FnBodyCtx<'_>,
        with_arrow: bool,
    ) -> Result<(), Error> {
        let (wrap_do, child_monadic) = match &arg.kind {
            ExpKind::Typ { exp, .. } if self.exp_has_effect(exp, it, ctx) => (true, true),
            ExpKind::Typ { .. } if self.exp_has_early_return(arg, it) => (false, false),
            ExpKind::Let { .. }
            | ExpKind::InternalPlet { .. }
            | ExpKind::If { .. }
            | ExpKind::Match { .. }
            | ExpKind::Var { .. }
            | ExpKind::Block(_) => {
                if self.exp_has_effect(arg, it, ctx) {
                    (true, true)
                } else {
                    (false, false)
                }
            }
            _ if self.exp_has_loop(arg, it) => (true, true),
            _ => (false, !with_arrow),
        };
        // (OCaml returns the identity wrapper plus `not with_arrow` for
        // plain arguments: inside `sailTryCatch` the argument stays a
        // computation.)
        if wrap_do {
            let mut inner = String::new();
            self.print_exp_inner(&mut inner, arg, it, ctx, child_monadic)?;
            if with_arrow {
                write!(out, "(← do {inner})")?;
            } else {
                write!(out, "(do {inner})")?;
            }
            return Ok(());
        }
        if child_monadic {
            return self.print_exp_inner(out, arg, it, ctx, true);
        }
        // Pure argument in a strict position: OCaml leaves plain
        // arguments unwrapped here (`(fun x -> x)`); parenthesise only
        // the sequencing forms to preserve evaluation order.
        let needs_parens = matches!(
            &arg.kind,
            ExpKind::Let { .. }
                | ExpKind::InternalPlet { .. }
                | ExpKind::If { .. }
                | ExpKind::Match { .. }
                | ExpKind::Var { .. }
                | ExpKind::Block(_)
                | ExpKind::Typ { .. }
        );
        if needs_parens {
            out.write_char('(')?;
            self.print_exp_inner(out, arg, it, ctx, false)?;
            out.write_char(')')?;
        } else {
            self.print_exp_inner(out, arg, it, ctx, false)?;
        }
        Ok(())
    }

    /// Render an argument fragment (for `early_return`/`throw`/extern
    /// call sites taking a pre-wrapped `d_of_arg`).
    fn render_arg<I: Interner>(
        &self,
        arg: &AstExp,
        it: &I,
        ctx: &FnBodyCtx<'_>,
        with_arrow: bool,
    ) -> Result<String, Error> {
        let mut buf = String::new();
        self.print_arg(&mut buf, arg, it, ctx, with_arrow)?;
        Ok(buf)
    }

    /// `(pure …)` in monadic position, bare otherwise (`wrap_with_pure`).
    fn with_pure(out: &mut String, as_monadic: bool, inner: &str) -> Result<(), Error> {
        if as_monadic {
            write!(out, "(pure {inner})")?;
        } else {
            out.push_str(inner);
        }
        Ok(())
    }

    /// `(← …)` outside monadic position (`wrap_with_left_arrow`).
    fn with_arrow(out: &mut String, needs_arrow: bool, inner: &str) -> Result<(), Error> {
        if needs_arrow {
            write!(out, "(← {inner})")?;
        } else {
            out.push_str(inner);
        }
        Ok(())
    }

    /// Syntactic `has_loop`: loop-combinator applications
    /// (`while#`/`while#t`/`foreach#`/`until#`/`until#t`, `is_loop`),
    /// direct loops/`for`, and returns.
    fn exp_has_loop<I: Interner>(&self, exp: &AstExp, it: &I) -> bool {
        match &exp.kind {
            ExpKind::Loop { .. } | ExpKind::For { .. } => true,
            ExpKind::Return(_) | ExpKind::InternalReturn(_) => true,
            ExpKind::App { func, args } => {
                self.app_is_loop(func, it) || args.iter().any(|a| self.exp_has_loop(a, it))
            }
            ExpKind::Block(es) => es.iter().any(|a| self.exp_has_loop(a, it)),
            ExpKind::Typ { exp, .. } => self.exp_has_loop(exp, it),
            ExpKind::Tuple(es) | ExpKind::Vector(es) | ExpKind::List(es) => {
                es.iter().any(|a| self.exp_has_loop(a, it))
            }
            ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
                self.exp_has_loop(a, it) || self.exp_has_loop(b, it)
            }
            ExpKind::If { cond, then_branch, else_branch } => {
                self.exp_has_loop(cond, it) || self.exp_has_loop(then_branch, it) || self.exp_has_loop(else_branch, it)
            }
            ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
                self.exp_has_loop(bind, it) || self.exp_has_loop(body, it)
            }
            ExpKind::Match { scrutinee, arms } => {
                self.exp_has_loop(scrutinee, it)
                    || arms.iter().any(|a| match &a.kind {
                        PExpKind::PatExp { body, .. } => self.exp_has_loop(body, it),
                        PExpKind::PatWhen { guard, body, .. } => {
                            self.exp_has_loop(guard, it) || self.exp_has_loop(body, it)
                        }
                    })
            }
            ExpKind::Try { exp, arms } => {
                self.exp_has_loop(exp, it)
                    || arms.iter().any(|a| match &a.kind {
                        PExpKind::PatExp { body, .. } => self.exp_has_loop(body, it),
                        PExpKind::PatWhen { guard, body, .. } => {
                            self.exp_has_loop(guard, it) || self.exp_has_loop(body, it)
                        }
                    })
            }
            ExpKind::Var { init, body, .. } => self.exp_has_loop(init, it) || self.exp_has_loop(body, it),
            ExpKind::Struct { fields, .. } => fields.iter().any(|f| self.exp_has_loop(&f.value, it)),
            ExpKind::StructUpdate { record, fields } => {
                self.exp_has_loop(record, it) || fields.iter().any(|f| self.exp_has_loop(&f.value, it))
            }
            ExpKind::Field { record, .. } => self.exp_has_loop(record, it),
            ExpKind::Assert { cond, msg } => self.exp_has_loop(cond, it) || self.exp_has_loop(msg, it),
            _ => false,
        }
    }

    /// Syntactic `has_early_return` (ports `Rewriter.has_early_return`
    /// for canonical AST): surface `return` (elaborated to the
    /// `early_return` application) and internal `E_return` count; Sail
    /// exceptions (`E_throw`, `E_try`) and `E_exit` do not.
    fn exp_has_early_return<I: Interner>(&self, exp: &AstExp, it: &I) -> bool {
        match &exp.kind {
            ExpKind::Return(_) | ExpKind::InternalReturn(_) => true,
            ExpKind::App { func, args } => {
                name_text(func, it).map(|n| n == "early_return").unwrap_or(false)
                    || args.iter().any(|a| self.exp_has_early_return(a, it))
            }
            ExpKind::Throw(_) | ExpKind::Exit(_) => false,
            ExpKind::Block(es) => es.iter().any(|a| self.exp_has_early_return(a, it)),
            ExpKind::Typ { exp, .. } => self.exp_has_early_return(exp, it),
            ExpKind::Tuple(es) | ExpKind::Vector(es) | ExpKind::List(es) => {
                es.iter().any(|a| self.exp_has_early_return(a, it))
            }
            ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
                self.exp_has_early_return(a, it) || self.exp_has_early_return(b, it)
            }
            ExpKind::If { cond, then_branch, else_branch } => {
                self.exp_has_early_return(cond, it)
                    || self.exp_has_early_return(then_branch, it)
                    || self.exp_has_early_return(else_branch, it)
            }
            ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
                self.exp_has_early_return(bind, it) || self.exp_has_early_return(body, it)
            }
            ExpKind::Match { scrutinee, arms } | ExpKind::Try { exp: scrutinee, arms } => {
                self.exp_has_early_return(scrutinee, it)
                    || arms.iter().any(|a| match &a.kind {
                        PExpKind::PatExp { body, .. } => self.exp_has_early_return(body, it),
                        PExpKind::PatWhen { guard, body, .. } => {
                            self.exp_has_early_return(guard, it) || self.exp_has_early_return(body, it)
                        }
                    })
            }
            ExpKind::Var { init, body, .. } => {
                self.exp_has_early_return(init, it) || self.exp_has_early_return(body, it)
            }
            ExpKind::Struct { fields, .. } => fields.iter().any(|f| self.exp_has_early_return(&f.value, it)),
            ExpKind::StructUpdate { record, fields } => {
                self.exp_has_early_return(record, it)
                    || fields.iter().any(|f| self.exp_has_early_return(&f.value, it))
            }
            ExpKind::Field { record, .. } => self.exp_has_early_return(record, it),
            ExpKind::Assert { cond, msg } => {
                self.exp_has_early_return(cond, it) || self.exp_has_early_return(msg, it)
            }
            _ => false,
        }
    }

    /// Whether `exp` is effectful: register reads, writes, throwing or
    /// looping applications, calls into [`ExpCtx::effectful_fns`], and
    /// anything containing them. Mirrors the `has_effect`/`has_effect_app`
    /// guards (`is_effect_id`: `while#`/`foreach#`/`early_return`).
    fn exp_has_effect<I: Interner>(&self, exp: &AstExp, it: &I, ctx: &FnBodyCtx<'_>) -> bool {
        match &exp.kind {
            ExpKind::Assign { .. } => true,
            ExpKind::Throw(_) | ExpKind::Exit(_) => true,
            ExpKind::Return(_) | ExpKind::InternalReturn(_) => true,
            ExpKind::Loop { .. } | ExpKind::For { .. } => true,
            ExpKind::Id(name) => name_text(name, it)
                .map(|n| ctx.env.registers.contains(&n))
                .unwrap_or(false),
            ExpKind::App { func, args } => {
                let is_effect_app = name_text(func, it)
                    .map(|n| {
                        n == "while#" || n == "foreach#" || n == "early_return" || ctx.env.effectful_fns.contains(&n)
                    })
                    .unwrap_or(false);
                is_effect_app || args.iter().any(|a| self.exp_has_effect(a, it, ctx))
            }
            ExpKind::Block(es) => es.iter().any(|e| self.exp_has_effect(e, it, ctx)),
            ExpKind::Typ { exp, .. } => self.exp_has_effect(exp, it, ctx),
            ExpKind::Tuple(es) | ExpKind::Vector(es) | ExpKind::List(es) => {
                es.iter().any(|e| self.exp_has_effect(e, it, ctx))
            }
            ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
                self.exp_has_effect(a, it, ctx) || self.exp_has_effect(b, it, ctx)
            }
            ExpKind::If { cond, then_branch, else_branch } => {
                self.exp_has_effect(cond, it, ctx)
                    || self.exp_has_effect(then_branch, it, ctx)
                    || self.exp_has_effect(else_branch, it, ctx)
            }
            ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
                self.exp_has_effect(bind, it, ctx) || self.exp_has_effect(body, it, ctx)
            }
            ExpKind::Match { scrutinee, arms } => {
                self.exp_has_effect(scrutinee, it, ctx)
                    || arms.iter().any(|a| match &a.kind {
                        PExpKind::PatExp { body, .. } => self.exp_has_effect(body, it, ctx),
                        PExpKind::PatWhen { guard, body, .. } => {
                            self.exp_has_effect(guard, it, ctx) || self.exp_has_effect(body, it, ctx)
                        }
                    })
            }
            ExpKind::Try { exp, arms } => {
                self.exp_has_effect(exp, it, ctx)
                    || arms.iter().any(|a| match &a.kind {
                        PExpKind::PatExp { body, .. } => self.exp_has_effect(body, it, ctx),
                        PExpKind::PatWhen { guard, body, .. } => {
                            self.exp_has_effect(guard, it, ctx) || self.exp_has_effect(body, it, ctx)
                        }
                    })
            }
            ExpKind::Var { init, body, .. } => {
                self.exp_has_effect(init, it, ctx) || self.exp_has_effect(body, it, ctx)
            }
            ExpKind::Struct { fields, .. } => fields.iter().any(|f| self.exp_has_effect(&f.value, it, ctx)),
            ExpKind::StructUpdate { record, fields } => {
                self.exp_has_effect(record, it, ctx)
                    || fields.iter().any(|f| self.exp_has_effect(&f.value, it, ctx))
            }
            ExpKind::Field { record, .. } => self.exp_has_effect(record, it, ctx),
            ExpKind::Assert { cond, msg } => {
                self.exp_has_effect(cond, it, ctx) || self.exp_has_effect(msg, it, ctx)
            }
            ExpKind::Ref(_) | ExpKind::Lit(_) | ExpKind::Undef => false,
            ExpKind::Sizeof(_)
            | ExpKind::Config(_)
            | ExpKind::Constraint(_)
            | ExpKind::InternalValue(_)
            | ExpKind::InternalAssume { .. } => false,
        }
    }

    /// Check whether an application head names a loop combinator
    /// (`while#`/`while#t`/`foreach#`/`until#`/`until#t`, `is_loop`).
    fn app_is_loop<I: Interner>(&self, func: &Name, it: &I) -> bool {
        name_text(func, it)
            .map(|n| matches!(n.as_str(), "while#" | "while#t" | "foreach#" | "until#" | "until#t"))
            .unwrap_or(false)
    }

    /// Main expression printer: port of `doc_exp`.
    fn print_exp_inner<W: fmt::Write, I: Interner>(
        &self,
        out: &mut W,
        exp: &AstExp,
        it: &I,
        ctx: &FnBodyCtx<'_>,
        as_monadic: bool,
    ) -> Result<(), Error> {
        // Buffered rendering: wrappers (`pure`, arrows, parens) apply
        // around a finished inner fragment.
        let mut inner = String::new();
        match &exp.kind {
            ExpKind::Id(name) => {
                let id = name_text(name, it)?;
                if ctx.env.registers.contains(id.as_str()) {
                    let mut id_buf = String::new();
                    self.print_id_ctor(&mut id_buf, id.as_str(), false)?;
                    let frag = format!("readReg {id_buf}");
                    Self::with_arrow(&mut inner, !as_monadic, &frag)?;
                } else {
                    let mut id_buf = String::new();
                    self.print_id_ctor(&mut id_buf, &id, false)?;
                    Self::with_pure(&mut inner, as_monadic, &id_buf)?;
                }
            }
            ExpKind::Lit(l) => {
                let lit = crate::conv::lit_from_ast(l, it).ok_or_else(|| Error::Unsupported {
                    msg: "literal is not translatable (real literals need the Real support module)".to_string(),
                    span: exp.span,
                })?;
                let mut lit_buf = String::new();
                self.print_lit(&mut lit_buf, &lit)?;
                Self::with_pure(&mut inner, as_monadic, &lit_buf)?;
            }
            ExpKind::App { func, args } => {
                let fname = name_text(func, it)?;
                self.print_app(&mut inner, exp, &fname, func, args, it, ctx, as_monadic)?;
            }
            ExpKind::Tuple(es) => {
                let mut tup = String::new();
                tup.push('(');
                for (i, e) in es.iter().enumerate() {
                    if i > 0 {
                        tup.push_str(", ");
                    }
                    let frag = self.render_arg(e, it, ctx, true)?;
                    tup.push_str(&frag);
                }
                tup.push(')');
                Self::with_pure(&mut inner, as_monadic, &tup)?;
            }
            ExpKind::Typ { typ, exp: inner_exp } => {
                // Ascribed vector literals dispatch on the syntactic
                // type (`is_bitvector_typ` needs the typechecker; the
                // `E_typ` ascription is the only type available here).
                if let ExpKind::Vector(vals) = &inner_exp.kind {
                    let lowered = typ_from_ast(typ, it).ok_or_else(|| Error::Unsupported {
                        msg: "vector literal ascription is not translatable".to_string(),
                        span: exp.span,
                    })?;
                    let frag = self.render_typed_vector(vals, &lowered, it, ctx)?;
                    Self::with_pure(&mut inner, as_monadic, &frag)?;
                } else if self.exp_has_effect(inner_exp, it, ctx) {
                    let frag = self.render_exp(inner_exp, it, ctx, as_monadic)?;
                    inner.push_str(&frag);
                } else {
                    let mut val = String::new();
                    self.print_exp_inner(&mut val, inner_exp, it, ctx, false)?;
                    let mut ty = String::new();
                    self.print_typ(&mut ty, &typ_from_ast(typ, it).ok_or_else(|| {
                        Error::Unsupported {
                            msg: "type ascription is not translatable".to_string(),
                            span: exp.span,
                        }
                    })?)?;
                    let frag = format!("({val} : {ty})");
                    Self::with_pure(&mut inner, as_monadic, &frag)?;
                }
            }
            ExpKind::Vector(_) => {
                return Err(Error::Unsupported {
                    msg: "bare vector literal is not translatable (needs an E_typ ascription to choose BitVec.join1 vs #v[…])"
                        .to_string(),
                    span: exp.span,
                });
            }
            ExpKind::Let { pat, bind, body } | ExpKind::InternalPlet { pat, bind, body } => {
                let is_internal = matches!(&exp.kind, ExpKind::InternalPlet { .. });
                let frag = self.render_let(pat, bind, body, is_internal, it, ctx, as_monadic)?;
                inner.push_str(&frag);
            }
            ExpKind::InternalReturn(e) => {
                let frag = self.render_exp(e, it, ctx, false)?;
                inner.push_str(&frag);
            }
            ExpKind::Return(e) => {
                // Elaborated `early_return` application (superset of
                // OCaml, same `SailME.throw`/`throw` semantics).
                let frag = self.render_arg(e, it, ctx, true)?;
                inner.push_str(Self::throw_name(ctx.in_sail_monad));
                inner.push(' ');
                inner.push_str(&frag);
            }
            ExpKind::Struct { fields, .. } => {
                let frag = self.render_record(fields, None, it, ctx, as_monadic)?;
                inner.push_str(&frag);
            }
            ExpKind::StructUpdate { record, fields } => {
                let base = self.render_exp(record, it, ctx, false)?;
                let frag = self.render_record(fields, Some(&base), it, ctx, as_monadic)?;
                inner.push_str(&frag);
            }
            ExpKind::Field { record, field } => {
                let base = self.render_exp(record, it, ctx, false)?;
                let fname = name_text(field, it)?;
                let mut field_buf = String::new();
                self.print_id_ctor(&mut field_buf, fname.as_str(), false)?;
                let frag = format!("{base}.{field_buf}");
                Self::with_pure(&mut inner, as_monadic, &frag)?;
            }
            ExpKind::Match { scrutinee, arms } => {
                let frag = self.render_match(scrutinee, arms, it, ctx, as_monadic)?;
                inner.push_str(&frag);
            }
            ExpKind::Assign { lexp, value } => {
                let frag = self.render_assign(lexp, value, it, ctx)?;
                Self::with_arrow(&mut inner, !as_monadic, &frag)?;
            }
            ExpKind::If { cond, then_branch, else_branch } => {
                let statements_monadic =
                    as_monadic || self.exp_has_effect(then_branch, it, ctx) || self.exp_has_effect(else_branch, it, ctx);
                let cond_frag = self.render_arg(cond, it, ctx, true)?;
                let then_frag = self.render_wrapped(then_branch, it, ctx, statements_monadic)?;
                let else_frag = self.render_wrapped(else_branch, it, ctx, statements_monadic)?;
                let frag = format!("if ({cond_frag} : Bool)\nthen {then_frag}\nelse {else_frag}");
                Self::with_arrow(&mut inner, statements_monadic && !as_monadic, &frag)?;
            }
            ExpKind::Ref(name) => {
                let id = name_text(name, it)?;
                let mut id_buf = String::new();
                self.print_id_ctor(&mut id_buf, id.as_str(), false)?;
                inner.push_str(&format!("(.Reg {id_buf})"));
            }
            ExpKind::Exit(_) => {
                inner.push_str(sail_except::render::render_exit());
            }
            ExpKind::Throw(e) => {
                let payload = self.render_exp(e, it, ctx, false)?;
                inner.push_str(&sail_except::render::render_throw(as_monadic, &payload));
            }
            ExpKind::Try { exp: tried, arms } => {
                let tried_frag = self.render_arg(tried, it, ctx, false)?;
                let arms_frag = self.render_try_arms(arms, it, ctx)?;
                let tried_er = self.exp_has_early_return(tried, it);
                inner.push_str(&sail_except::render::render_try_catch(
                    as_monadic,
                    tried_er,
                    &tried_frag,
                    &arms_frag,
                ));
            }
            ExpKind::Assert { cond, msg } => {
                let cond_frag = self.render_arg(cond, it, ctx, true)?;
                let msg_frag = self.render_arg(msg, it, ctx, true)?;
                inner.push_str(&sail_except::render::render_assert(&cond_frag, &msg_frag));
            }
            ExpKind::List(es) => {
                let mut list = String::new();
                list.push('[');
                for (i, e) in es.iter().enumerate() {
                    if i > 0 {
                        list.push_str(", ");
                    }
                    list.push_str(&self.render_exp(e, it, ctx, as_monadic)?);
                }
                list.push(']');
                inner.push_str(&list);
            }
            ExpKind::Cons(hd, tl) => {
                let hd_frag = self.render_exp(hd, it, ctx, false)?;
                let tl_frag = self.render_exp(tl, it, ctx, false)?;
                let frag = format!("({hd_frag} :: {tl_frag})");
                inner.push_str(&frag);
            }
            ExpKind::For { var, from, to, step, order, body } => {
                let frag = self.render_for(var, from, to, step, *order, body, it, ctx)?;
                inner.push_str(&frag);
            }
            ExpKind::VectorAppend(_, _)
            | ExpKind::Loop { .. }
            | ExpKind::Block(_)
            | ExpKind::Var { .. }
            | ExpKind::Sizeof(_)
            | ExpKind::Constraint(_)
            | ExpKind::Config(_)
            | ExpKind::Undef
            | ExpKind::InternalValue(_)
            | ExpKind::InternalAssume { .. } => {
                return Err(Error::Unsupported {
                    msg: format!("expression {} is not translatable yet", exp_con(exp)),
                    span: exp.span,
                });
            }
        }
        out.write_str(&inner)?;
        Ok(())
    }

    /// Render an expression fragment in the given monadic position.
    fn render_exp<I: Interner>(
        &self,
        exp: &AstExp,
        it: &I,
        ctx: &FnBodyCtx<'_>,
        as_monadic: bool,
    ) -> Result<String, Error> {
        let mut buf = String::new();
        self.print_exp_inner(&mut buf, exp, it, ctx, as_monadic)?;
        Ok(buf)
    }

    /// `wrap_exp` (`pretty_print_lean.ml#~745-760`): arms and branches
    /// gaining `do`/`(…)` by shape.
    fn render_wrapped<I: Interner>(
        &self,
        exp: &AstExp,
        it: &I,
        ctx: &FnBodyCtx<'_>,
        as_monadic: bool,
    ) -> Result<String, Error> {
        let with_arrow = !as_monadic;
        match &exp.kind {
            ExpKind::Typ { exp: inner, .. } if self.exp_has_effect(inner, it, ctx) => {
                let body = self.render_exp(exp, it, ctx, as_monadic)?;
                if with_arrow {
                    Ok(format!("(← do {body})"))
                } else {
                    Ok(format!("(do {body})"))
                }
            }
            ExpKind::Typ { .. } if self.exp_has_early_return(exp, it) => {
                Ok(format!("({})", self.render_exp(exp, it, ctx, as_monadic)?))
            }
            ExpKind::Let { .. }
            | ExpKind::InternalPlet { .. }
            | ExpKind::If { .. }
            | ExpKind::Match { .. }
            | ExpKind::Var { .. }
            | ExpKind::Block(_) => {
                if self.exp_has_effect(exp, it, ctx) {
                    let body = self.render_exp(exp, it, ctx, as_monadic)?;
                    if with_arrow {
                        Ok(format!("(← do {body})"))
                    } else {
                        Ok(format!("(do {body})"))
                    }
                } else {
                    Ok(format!("({})", self.render_exp(exp, it, ctx, as_monadic)?))
                }
            }
            _ if self.exp_has_loop(exp, it) => {
                let body = self.render_exp(exp, it, ctx, as_monadic)?;
                if with_arrow {
                    Ok(format!("(← do {body})"))
                } else {
                    Ok(format!("(do {body})"))
                }
            }
            _ => self.render_exp(exp, it, ctx, as_monadic),
        }
    }

    /// Application cases: `None`/`Some`/`__id`/combinators/`early_return`,
    /// extern operators, and plain calls.
    #[allow(clippy::too_many_arguments)]
    fn print_app<W: fmt::Write, I: Interner>(
        &self,
        out: &mut W,
        exp: &AstExp,
        fname: &str,
        func: &Name,
        args: &[AstExp],
        it: &I,
        ctx: &FnBodyCtx<'_>,
        as_monadic: bool,
    ) -> Result<(), Error> {
        let mut inner = String::new();
        match (fname, args) {
            ("None", _) => {
                Self::with_pure(&mut inner, as_monadic, "none")?;
            }
            ("Some", _) => {
                let mut app = String::from("(some");
                for a in args {
                    app.push(' ');
                    app.push_str(&self.render_arg(a, it, ctx, true)?);
                }
                app.push(')');
                Self::with_pure(&mut inner, as_monadic, &app)?;
            }
            ("__id", [e]) => {
                inner.push_str(&self.render_exp(e, it, ctx, as_monadic)?);
            }
            ("early_return", [arg]) => {
                let frag = self.render_arg(arg, it, ctx, true)?;
                inner.push_str(Self::throw_name(ctx.in_sail_monad));
                inner.push(' ');
                inner.push_str(&frag);
            }
            _ if self.app_is_loop(func, it) || fname == "early_return" => {
                return Err(Error::Unsupported {
                    msg: format!(
                        "loop combinator `{fname}` needs loop-variable desugaring (doc_loop), which is not implemented"
                    ),
                    span: exp.span,
                });
            }
            _ => {
                let extern_spelling = ctx.env.lean_externs.get(fname).map(String::as_str);
                match lean_binop_of_extern(extern_spelling) {
                    ExternOp::Binop(op) if args.len() == 2 => {
                        let lhs = self.render_arg(&args[0], it, ctx, true)?;
                        let rhs = self.render_arg(&args[1], it, ctx, true)?;
                        let frag = format!("({lhs} {op} {rhs})");
                        Self::with_pure(&mut inner, as_monadic, &frag)?;
                    }
                    ExternOp::Unop(op) if args.len() == 1 => {
                        let arg = self.render_arg(&args[0], it, ctx, true)?;
                        let frag = format!("({op} {arg})");
                        Self::with_pure(&mut inner, as_monadic, &frag)?;
                    }
                    _ => {
                        if func.is_operator && extern_spelling.is_none() {
                            return Err(Error::Unsupported {
                                msg: format!(
                                    "operator `{fname}` has no lean-extern spelling (needs the pragma lean(...) table)"
                                ),
                                span: exp.span,
                            });
                        }
                        if extern_spelling.is_some() && args.is_empty() {
                            return Err(Error::Unsupported {
                                msg: format!("extern `{fname}` application has no arguments"),
                                span: func.span,
                            });
                        }
                        let fn_monadic = ctx.env.effectful_fns.contains(fname);
                        let mut id_buf = String::new();
                        self.print_id_ctor(&mut id_buf, fname, true)?;
                        let mut app = id_buf;
                        for a in args {
                            app.push(' ');
                            app.push_str(&self.render_arg(a, it, ctx, true)?);
                        }
                        let called = format!("({app})");
                        let mut wrapped = String::new();
                        Self::with_pure(&mut wrapped, as_monadic && !fn_monadic, &called)?;
                        inner.clear();
                        Self::with_arrow(&mut inner, !as_monadic && fn_monadic, &wrapped)?;
                    }
                }
            }
        }
        out.write_str(&inner)?;
        Ok(())
    }

    /// Type-directed vector literal (`E_vector` under `E_typ`):
    /// bitvectors join with `BitVec.join1`, other vectors print reversed
    /// under `#v[…]` (both mirror `doc_exp`, ~lines 965-973).
    fn render_typed_vector<I: Interner>(
        &self,
        vals: &[AstExp],
        lowered: &Typ,
        it: &I,
        ctx: &FnBodyCtx<'_>,
    ) -> Result<String, Error> {
        match lowered {
            Typ::BitVec(_) | Typ::Bit(_) => {
                let mut out = String::from("(BitVec.join1 [");
                for (i, v) in vals.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&self.render_arg(v, it, ctx, true)?);
                }
                out.push_str("])");
                Ok(out)
            }
            _ => {
                let mut out = String::from("#v[");
                for (i, v) in vals.iter().rev().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&self.render_arg(v, it, ctx, true)?);
                }
                out.push(']');
                Ok(out)
            }
        }
    }

    /// `let`/`internal_plet` (`doc_exp`, ~lines 978-1022, simplified):
    /// effectful right-hand sides bind with `←`/`← do`, pure ones with
    /// `:=`, typed ones gain the `prepend_monad` ascription. A unit
    /// literal into an anonymous pattern emits the bare right-hand side.
    fn render_let<I: Interner>(
        &self,
        pat: &sail_ast::Pat,
        bind: &AstExp,
        body: &AstExp,
        is_internal: bool,
        it: &I,
        ctx: &FnBodyCtx<'_>,
        as_monadic: bool,
    ) -> Result<String, Error> {
        let lowered = pat_from_ast(pat, it).ok_or_else(|| Error::Unsupported {
            msg: "let pattern is not translatable".to_string(),
            span: pat.span,
        })?;
        let mut pat_buf = String::new();
        self.print_pat_top(&mut pat_buf, &lowered)?;
        let ascription = Self::pat_typ_ascription(&lowered);
        let mut asc_buf = String::new();
        if let Some(ty) = ascription {
            self.print_typ(&mut asc_buf, ty)?;
        }
        let effectful = self.exp_has_effect(bind, it, ctx);
        let has_loop = self.exp_has_loop(bind, it);
        let is_unit_lit = matches!(
            &bind.kind,
            ExpKind::Lit(l) if matches!(&l.kind, sail_ast::LitKind::Unit)
        );
        if effectful || has_loop {
            if is_unit_lit && pat_is_anonymous(&lowered) {
                let rhs = self.render_exp(bind, it, ctx, true)?;
                let rest = self.render_exp(body, it, ctx, as_monadic)?;
                return Ok(format!("{rhs}\n{rest}"));
            }
            // `is_arrow_do`: flat `let` chains use `←`, everything else
            // (`internal_plet`, loop bodies) uses `← do`.
            let arrow = if !is_internal && !has_loop { "←" } else { "← do" };
            let rhs = self.render_exp(bind, it, ctx, true)?;
            let rest = self.render_exp(body, it, ctx, as_monadic)?;
            if ascription.is_some() {
                // `prepend_monad` ascription (`pretty_print_lean.ml#725-730`):
                // the full OCaml form prefixes both the function return
                // type and the ascribed type (`SailME <ret> <ascribed>`),
                // but the untyped AST carries no return type — ascribe the
                // computation with the monad head and the pattern type.
                let mut head = String::new();
                self.print_ascribed_monad_head(&mut head, ctx.in_sail_monad, ctx.in_except_monad)?;
                let rest_ty = asc_buf;
                return Ok(format!("let {pat_buf} {arrow} (({rhs}) : {head} {rest_ty})\n{rest}"));
            }
            return Ok(format!("let {pat_buf} {arrow} {rhs}\n{rest}"));
        }
        let rhs = self.render_exp(bind, it, ctx, false)?;
        let rest = self.render_exp(body, it, ctx, as_monadic)?;
        if ascription.is_some() {
            Ok(format!("let {pat_buf} : {asc_buf} := {rhs}\n{rest}"))
        } else {
            Ok(format!("let {pat_buf} := {rhs}\n{rest}"))
        }
    }

    /// `prepend_monad` head for ascribed effectful lets
    /// (`pretty_print_lean.ml#725-730`): `SailME`/`SailM`/`ExceptM`/`Id`.
    fn print_ascribed_monad_head<W: fmt::Write>(
        &self,
        out: &mut W,
        in_sail_monad: bool,
        in_except_monad: bool,
    ) -> Result<(), Error> {
        match (in_sail_monad, in_except_monad) {
            (true, true) => out.write_str("SailME")?,
            (true, false) => out.write_str("SailM")?,
            (false, true) => out.write_str("ExceptM")?,
            (false, false) => out.write_str("Id")?,
        }
        Ok(())
    }

    /// Record literal / update (`E_struct`, `E_struct_update`).
    fn render_record<I: Interner>(
        &self,
        fields: &[AstFExp],
        base: Option<&str>,
        it: &I,
        ctx: &FnBodyCtx<'_>,
        as_monadic: bool,
    ) -> Result<String, Error> {
        let mut out = String::from("{ ");
        if let Some(record) = base {
            out.push_str(record);
            out.push_str(" with ");
        }
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            let fname = name_text(&f.field, it)?;
            let mut id_buf = String::new();
            self.print_id_ctor(&mut id_buf, &fname, false)?;
            out.push_str(&id_buf);
            out.push_str(" := ");
            if self.exp_has_effect(&f.value, it, ctx) {
                out.push_str("← ");
            }
            out.push_str(&self.render_exp(&f.value, it, ctx, self.exp_has_effect(&f.value, it, ctx))?);
        }
        out.push_str(" }");
        let mut wrapped = String::new();
        Self::with_pure(&mut wrapped, as_monadic, &out)?;
        Ok(wrapped)
    }

    /// `E_match` (incl. `match_bv` selection via [`is_match_bv`]).
    fn render_match<I: Interner>(
        &self,
        scrutinee: &AstExp,
        arms: &[AstPExp],
        it: &I,
        ctx: &FnBodyCtx<'_>,
        as_monadic: bool,
    ) -> Result<String, Error> {
        let mut lowered: Vec<(Pat, &AstPExp)> = Vec::with_capacity(arms.len());
        for arm in arms {
            let (pat_node, _) = match &arm.kind {
                PExpKind::PatExp { pat, .. } => (pat, true),
                PExpKind::PatWhen { pat, .. } => (pat, false),
            };
            let pat = pat_from_ast(pat_node, it).ok_or_else(|| Error::Unsupported {
                msg: "match pattern is not translatable".to_string(),
                span: pat_node.span,
            })?;
            lowered.push((pat, arm));
        }
        let arm_pats: Vec<Pat> = lowered.iter().map(|(p, _)| p.clone()).collect();
        let is_bv = is_match_bv(&arm_pats);
        let as_monadic_prime = arms.iter().any(|a| match &a.kind {
            PExpKind::PatExp { body, .. } => self.exp_has_effect(body, it, ctx),
            PExpKind::PatWhen { guard, body, .. } => {
                self.exp_has_effect(guard, it, ctx) || self.exp_has_effect(body, it, ctx)
            }
        }) || as_monadic;
        let keyword = if is_bv { "match_bv " } else { "match " };
        let mut out = String::new();
        out.push_str(keyword);
        out.push_str(&self.render_arg(scrutinee, it, ctx, true)?);
        out.push_str(" with");
        for (pat, arm) in &lowered {
            let mut pat_buf = String::new();
            self.print_pat(&mut pat_buf, pat, is_bv)?;
            match &arm.kind {
                PExpKind::PatExp { body, .. } => {
                    out.push_str(&format!("\n| {pat_buf} =>"));
                    if is_bv && as_monadic_prime {
                        out.push_str(" do");
                    }
                    out.push(' ');
                    out.push_str(&self.render_wrapped(body, it, ctx, as_monadic_prime)?);
                }
                PExpKind::PatWhen { guard, body, .. } => {
                    if !is_bv {
                        return Err(Error::Unsupported {
                            msg: "when-clause in a non-bitvector match is not translatable".to_string(),
                            span: arm.span,
                        });
                    }
                    let guard_frag = self.render_exp(guard, it, ctx, false)?;
                    out.push_str(&format!("\n| {pat_buf} if {guard_frag} =>"));
                    if as_monadic_prime {
                        out.push_str(" do");
                    }
                    out.push(' ');
                    out.push_str(&self.render_wrapped(body, it, ctx, as_monadic_prime)?);
                }
            }
        }
        Ok(out)
    }

    /// `E_try` arms: `match the_exception with …` in monadic position.
    fn render_try_arms<I: Interner>(
        &self,
        arms: &[AstPExp],
        it: &I,
        ctx: &FnBodyCtx<'_>,
    ) -> Result<String, Error> {
        let mut lowered: Vec<Pat> = Vec::with_capacity(arms.len());
        for arm in arms {
            let pat_node = match &arm.kind {
                PExpKind::PatExp { pat, .. } => pat,
                PExpKind::PatWhen { pat, .. } => pat,
            };
            lowered.push(pat_from_ast(pat_node, it).ok_or_else(|| Error::Unsupported {
                msg: "try pattern is not translatable".to_string(),
                span: pat_node.span,
            })?);
        }
        let is_bv = is_match_bv(&lowered);
        if lowered.iter().zip(arms).any(|(_, arm)| matches!(&arm.kind, PExpKind::PatWhen { .. }) && !is_bv) {
            let bad = arms
                .iter()
                .find(|a| matches!(&a.kind, PExpKind::PatWhen { .. }))
                .map(|a| a.span)
                .unwrap_or(crate::Span::DUMMY);
            return Err(Error::Unsupported {
                msg: "when-clause in a non-bitvector try is not translatable".to_string(),
                span: bad,
            });
        }
        let keyword = if is_bv { "match_bv " } else { "match " };
        let mut out = String::new();
        out.push_str(keyword);
        out.push_str("the_exception with");
        for (pat, arm) in lowered.iter().zip(arms) {
            let mut pat_buf = String::new();
            self.print_pat(&mut pat_buf, pat, is_bv)?;
            match &arm.kind {
                PExpKind::PatExp { body, .. } => {
                    out.push_str(&format!("\n| {pat_buf} => "));
                    out.push_str(&self.render_wrapped(body, it, ctx, true)?);
                }
                PExpKind::PatWhen { guard, body, .. } => {
                    let guard_frag = self.render_exp(guard, it, ctx, false)?;
                    out.push_str(&format!("\n| {pat_buf} if {guard_frag} => "));
                    out.push_str(&self.render_wrapped(body, it, ctx, true)?);
                }
            }
        }
        Ok(out)
    }

    /// Register write (`E_assign` for `LE_id`/`LE_typ`/`LE_deref`).
    fn render_assign<I: Interner>(
        &self,
        lexp: &AstLExp,
        value: &AstExp,
        it: &I,
        ctx: &FnBodyCtx<'_>,
    ) -> Result<String, Error> {
        match &lexp.kind {
            LExpKind::Id(name) | LExpKind::Typ { id: name, .. } => {
                let id = name_text(name, it)?;
                let mut id_buf = String::new();
                self.print_id_ctor(&mut id_buf, &id, false)?;
                let val = self.render_arg(value, it, ctx, true)?;
                Ok(format!("writeReg {id_buf} {val}"))
            }
            LExpKind::Deref(target) => {
                let base = self.render_arg(target, it, ctx, true)?;
                let val = self.render_arg(value, it, ctx, true)?;
                Ok(format!("writeRegRef {base} {val}"))
            }
            _ => Err(Error::Unsupported {
                msg: format!("assignment target {} is not implemented", lexp_con(lexp)),
                span: lexp.span,
            }),
        }
    }

    /// Direct `E_for` (`pretty_print_lean.ml#919-934`): the elaborated
    /// loop keeps its `for … in […:…:…]i do` shape; a pure body runs
    /// under `Id.run`.
    fn render_for<I: Interner>(
        &self,
        var: &Name,
        from: &AstExp,
        to: &AstExp,
        step: &AstExp,
        order: Order,
        body: &AstExp,
        it: &I,
        ctx: &FnBodyCtx<'_>,
    ) -> Result<String, Error> {
        let vname = name_text(var, it)?;
        let mut var_buf = String::new();
        self.print_id_ctor(&mut var_buf, &vname, false)?;
        let from_frag = self.render_exp(from, it, ctx, false)?;
        let to_frag = self.render_exp(to, it, ctx, false)?;
        let step_frag = self.render_exp(step, it, ctx, false)?;
        let step_frag = match order {
            Order::Inc => step_frag,
            Order::Dec => format!("-{step_frag}"),
        };
        let body_effect = self.exp_has_effect(body, it, ctx);
        let prefix = if body_effect { "" } else { "Id.run " };
        let body_frag = self.render_exp(body, it, ctx, body_effect)?;
        Ok(format!(
            "{prefix}for {var_buf} in [{from_frag}:{to_frag}:{step_frag}]i do {body_frag}"
        ))
    }

}

/// Per-body monad flags (from the enclosing [`FunSig`]) plus the shared
/// environment sets ([`ExpCtx`]).
struct FnBodyCtx<'a> {
    in_sail_monad: bool,
    in_except_monad: bool,
    env: &'a ExpCtx,
}

/// Expression-constructor name for hard-error messages (mirrors
/// `string_of_exp_con`).
fn exp_con(exp: &AstExp) -> &'static str {
    match &exp.kind {
        ExpKind::Block(_) => "E_block",
        ExpKind::Id(_) => "E_id",
        ExpKind::Lit(_) => "E_lit",
        ExpKind::Typ { .. } => "E_typ",
        ExpKind::App { .. } => "E_app",
        ExpKind::Tuple(_) => "E_tuple",
        ExpKind::If { .. } => "E_if",
        ExpKind::Loop { .. } => "E_loop",
        ExpKind::For { .. } => "E_for",
        ExpKind::Vector(_) => "E_vector",
        ExpKind::VectorAppend(_, _) => "E_vector_append",
        ExpKind::List(_) => "E_list",
        ExpKind::Cons(_, _) => "E_cons",
        ExpKind::Struct { .. } => "E_struct",
        ExpKind::StructUpdate { .. } => "E_struct_update",
        ExpKind::Field { .. } => "E_field",
        ExpKind::Match { .. } => "E_match",
        ExpKind::Let { .. } => "E_let",
        ExpKind::Assign { .. } => "E_assign",
        ExpKind::Sizeof(_) => "E_sizeof",
        ExpKind::Return(_) => "E_return",
        ExpKind::Exit(_) => "E_exit",
        ExpKind::Config(_) => "E_config",
        ExpKind::Ref(_) => "E_ref",
        ExpKind::Throw(_) => "E_throw",
        ExpKind::Try { .. } => "E_try",
        ExpKind::Assert { .. } => "E_assert",
        ExpKind::Var { .. } => "E_var",
        ExpKind::Undef => "E_undef",
        ExpKind::InternalPlet { .. } => "E_internal_plet",
        ExpKind::InternalReturn(_) => "E_internal_return",
        ExpKind::InternalValue(_) => "E_internal_value",
        ExpKind::InternalAssume { .. } => "E_internal_assume",
        ExpKind::Constraint(_) => "E_constraint",
    }
}

/// Assignment-target name for hard-error messages.
fn lexp_con(lexp: &AstLExp) -> &'static str {
    match &lexp.kind {
        LExpKind::Id(_) => "LE_id",
        LExpKind::Deref(_) => "LE_deref",
        LExpKind::Typ { .. } => "LE_typ",
        LExpKind::Tuple(_) => "LE_tuple",
        LExpKind::VectorConcat(_) => "LE_vector_concat",
        LExpKind::Vector { .. } => "LE_vector",
        LExpKind::VectorRange { .. } => "LE_vector_range",
        LExpKind::Field { .. } => "LE_field",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use sail_ast::{
        ExpKind as AstExpKind, Lit as AstLit, LitKind as AstLitKind, Name as AstName, NodeId,
        Pat as AstPat, PatKind as AstPatKind, PExpKind as AstPExpKind, SimpleInterner, Typ as AstTyp,
        TypArg as AstTypArg, TypArgKind as AstTypArgKind, TypKind as AstTypKind,
    };

    struct Fixture {
        it: RefCell<SimpleInterner>,
    }

    impl Fixture {
        fn new() -> Self {
            Self { it: RefCell::new(SimpleInterner::new()) }
        }

        fn name(&self, text: &str) -> AstName {
            AstName::dummy(self.it.borrow_mut().intern(text))
        }

        fn opname(&self, text: &str) -> AstName {
            AstName::new(NodeId::DUMMY, crate::Span::DUMMY, self.it.borrow_mut().intern(text), true)
        }

        fn id(&self, text: &str) -> AstExp {
            let is_operator = text.bytes().next().is_some_and(|b| !(b.is_ascii_alphabetic() || b == b'_'));
            let symbol = self.it.borrow_mut().intern(text);
            let name = AstName::new(NodeId::DUMMY, crate::Span::DUMMY, symbol, is_operator);
            AstExp::dummy(AstExpKind::Id(name))
        }

        fn num(&self, n: &str) -> AstExp {
            let lit = AstLit::dummy(AstLitKind::Num(self.it.borrow_mut().intern(n)));
            AstExp::dummy(AstExpKind::Lit(lit))
        }

        fn app(&self, f: &str, args: Vec<AstExp>) -> AstExp {
            AstExp::dummy(AstExpKind::App { func: self.name(f), args })
        }

        fn pat_id(&self, text: &str) -> AstPat {
            AstPat::dummy(AstPatKind::Id(self.name(text)))
        }

        fn pat_wild(&self) -> AstPat {
            AstPat::dummy(AstPatKind::Wild)
        }

        fn nat_ty(&self) -> AstTyp {
            AstTyp::dummy(AstTypKind::Id(self.name("nat")))
        }

        fn intern(&self, text: &str) -> sail_ast::Symbol {
            self.it.borrow_mut().intern(text)
        }

        fn sig(&self, name: &str, monadic: bool) -> FunSig {
            FunSig {
                name: name.into(),
                args: vec![],
                ret: Typ::Nat,
                is_monadic: monadic,
                early_return: false,
                has_loop: false,
                span: crate::Span::DUMMY,
            }
        }
    }

    fn printer() -> LeanPrinter {
        LeanPrinter::with_default()
    }

    #[test]
    fn pure_call_body_prints_for_real() {
        let fx = Fixture::new();
        let body = fx.app("add", vec![fx.id("x"), fx.id("y")]);
        let got = printer().render_fun_def(&fx.sig("f", false), &body, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def f : Nat := (add x y)\n");
    }

    #[test]
    fn monadic_body_wraps_pure_values() {
        let fx = Fixture::new();
        let body = fx.app("add", vec![fx.id("x"), fx.num("1")]);
        let got = printer().render_fun_def(&fx.sig("f", true), &body, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def f : SailM Nat := do\n  (pure (add x 1))\n");
    }

    #[test]
    fn effectful_call_takes_arrow_in_pure_position() {
        let fx = Fixture::new();
        let mut ctx = ExpCtx::pure();
        ctx.effectful_fns.insert("tick".to_string());
        let body = fx.app("tick", vec![fx.id("x")]);
        let got = printer().render_fun_def(&fx.sig("f", false), &body, &*fx.it.borrow(), &ctx).unwrap();
        assert_eq!(got, "def f : Nat := (← (tick x))\n");
    }

    #[test]
    fn register_read_and_write() {
        let fx = Fixture::new();
        let mut ctx = ExpCtx::pure();
        ctx.registers.insert("r".to_string());
        // Read in a stateful body: bare `readReg`.
        let got = printer()
            .render_fun_def(&fx.sig("get", true), &fx.id("r"), &*fx.it.borrow(), &ctx)
            .unwrap();
        assert_eq!(got, "def get : SailM Nat := do\n  readReg r\n");
        // Read in a pure body: arrow-wrapped.
        let got = printer()
            .render_fun_def(&fx.sig("get", false), &fx.id("r"), &*fx.it.borrow(), &ctx)
            .unwrap();
        assert_eq!(got, "def get : Nat := (← readReg r)\n");
        // Write.
        let lexp = sail_ast::Node::dummy(LExpKind::Id(fx.name("r")));
        let assign = AstExp::dummy(ExpKind::Assign { lexp: Box::new(lexp), value: Box::new(fx.id("x")) });
        let got = printer().render_fun_def(&fx.sig("set", true), &assign, &*fx.it.borrow(), &ctx).unwrap();
        assert_eq!(got, "def set : SailM Nat := do\n  writeReg r x\n");
    }

    #[test]
    fn extern_binop_prints_infix() {
        let fx = Fixture::new();
        let mut ctx = ExpCtx::pure();
        ctx.lean_externs.insert("myadd".to_string(), "_lean_add".to_string());
        let body = fx.app("myadd", vec![fx.id("x"), fx.id("y")]);
        let got = printer().render_fun_def(&fx.sig("f", false), &body, &*fx.it.borrow(), &ctx).unwrap();
        assert_eq!(got, "def f : Nat := (x + y)\n");
    }

    #[test]
    fn operator_without_extern_is_hard_error() {
        let fx = Fixture::new();
        let body = AstExp::dummy(AstExpKind::App { func: fx.opname("+"), args: vec![fx.id("x"), fx.id("y")] });
        let err = printer().render_fun_def(&fx.sig("f", false), &body, &*fx.it.borrow(), &ExpCtx::pure()).unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        assert!(err.to_string().contains("lean-extern"));
    }

    #[test]
    fn let_if_match_print() {
        let fx = Fixture::new();
        // `let y := (f x) in y`.
        let bind = fx.app("f", vec![fx.id("x")]);
        let body = AstExp::dummy(AstExpKind::Let {
            pat: fx.pat_id("y"),
            bind: Box::new(bind),
            body: Box::new(fx.id("y")),
        });
        let got = printer().render_fun_def(&fx.sig("g", false), &body, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def g : Nat := let y := (f x)\ny\n");
        // `if c then 1 else 2`.
        let cond = fx.app("c", vec![]);
        let ite = AstExp::dummy(AstExpKind::If {
            cond: Box::new(cond),
            then_branch: Box::new(fx.num("1")),
            else_branch: Box::new(fx.num("2")),
        });
        let got = printer().render_fun_def(&fx.sig("h", false), &ite, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def h : Nat := if ((c) : Bool)\nthen 1\nelse 2\n");
    }

    #[test]
    fn ascribed_pure_let_prints_type() {
        let fx = Fixture::new();
        let typ = fx.nat_ty();
        let pat = AstPat::dummy(AstPatKind::Typ { typ, pat: Box::new(fx.pat_id("x")) });
        let body = AstExp::dummy(AstExpKind::Let {
            pat,
            bind: Box::new(fx.num("0")),
            body: Box::new(fx.id("x")),
        });
        let got = printer().render_fun_def(&fx.sig("a", false), &body, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def a : Nat := let x : Nat := 0\nx\n");
    }

    #[test]
    fn effectful_let_binds_with_arrow() {
        let fx = Fixture::new();
        let mut ctx = ExpCtx::pure();
        ctx.registers.insert("r".to_string());
        let bind = fx.id("r");
        let body = AstExp::dummy(AstExpKind::Let {
            pat: fx.pat_id("v"),
            bind: Box::new(bind),
            body: Box::new(fx.id("v")),
        });
        let got = printer().render_fun_def(&fx.sig("g", true), &body, &*fx.it.borrow(), &ctx).unwrap();
        assert_eq!(got, "def g : SailM Nat := do\n  let v ← readReg r\n(pure v)\n");
    }

    #[test]
    fn match_and_match_bv() {
        let fx = Fixture::new();
        // Plain option match.
        let some = AstPat::dummy(AstPatKind::App { func: fx.name("Some"), args: vec![fx.pat_id("y")] });
        let arm = AstPExp { id: NodeId::DUMMY, span: crate::Span::DUMMY, kind: AstPExpKind::PatExp {
            pat: some, body: fx.id("y"),
        } };
        let wild = AstPExp { id: NodeId::DUMMY, span: crate::Span::DUMMY, kind: AstPExpKind::PatExp {
            pat: fx.pat_wild(), body: fx.num("0"),
        } };
        let mat = AstExp::dummy(AstExpKind::Match { scrutinee: Box::new(fx.id("o")), arms: vec![arm, wild] });
        let got = printer().render_fun_def(&fx.sig("m", false), &mat, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def m : Nat := match o with\n| .some y => y\n| _ => 0\n");
        // Bitvector match selects `match_bv`.
        let b1 = AstLit::dummy(AstLitKind::Bin(fx.intern("1")));
        let b0 = AstLit::dummy(AstLitKind::Bin(fx.intern("0")));
        let bv = AstPat::dummy(AstPatKind::Vector(vec![
            AstPat::dummy(AstPatKind::Lit(b1)),
            AstPat::dummy(AstPatKind::Lit(b0)),
        ]));
        let arm = AstPExp { id: NodeId::DUMMY, span: crate::Span::DUMMY, kind: AstPExpKind::PatExp {
            pat: bv, body: fx.num("1"),
        } };
        let mat = AstExp::dummy(AstExpKind::Match { scrutinee: Box::new(fx.id("v")), arms: vec![arm] });
        let got = printer().render_fun_def(&fx.sig("n", false), &mat, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def n : Nat := match_bv v with\n| 10 => 1\n");
    }

    #[test]
    fn early_return_throw_exit_try_assert() {
        let fx = Fixture::new();
        // `return x` in SailM: `SailME.throw x`.
        let ret = AstExp::dummy(AstExpKind::Return(Box::new(fx.id("x"))));
        let got = printer().render_fun_def(&fx.sig("f", true), &ret, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def f : SailM Nat := do\n  SailME.throw x\n");
        // `throw x` in pure position: arrow.
        let throw = AstExp::dummy(AstExpKind::Throw(Box::new(fx.id("x"))));
        let got = printer().render_fun_def(&fx.sig("g", false), &throw, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def g : Nat := ←sailThrow (x)\n");
        // `exit`.
        let exit = AstExp::dummy(AstExpKind::Exit(Box::new(fx.num("0"))));
        let got = printer().render_fun_def(&fx.sig("e", true), &exit, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def e : SailM Nat := do\n  throw Error.Exit\n");
    }

    #[test]
    fn try_catch_selects_combinator() {
        let fx = Fixture::new();
        let tried = AstExp::dummy(AstExpKind::Throw(Box::new(fx.id("x"))));
        let arm = AstPExp { id: NodeId::DUMMY, span: crate::Span::DUMMY, kind: AstPExpKind::PatExp {
            pat: fx.pat_wild(), body: fx.num("0"),
        } };
        let attempt =
            AstExp::dummy(AstExpKind::Try { exp: Box::new(tried), arms: vec![arm] });
        let got = printer().render_fun_def(&fx.sig("t", true), &attempt, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert!(got.contains("sailTryCatch (sailThrow (x))"), "{got}");
        assert!(got.contains("match the_exception with"), "{got}");
    }

    #[test]
    fn for_loop_and_typed_vectors() {
        let fx = Fixture::new();
        let body = AstExp::dummy(AstExpKind::For {
            var: fx.name("i"),
            from: Box::new(fx.num("0")),
            to: Box::new(fx.num("10")),
            step: Box::new(fx.num("1")),
            order: Order::Inc,
            body: Box::new(fx.id("x")),
        });
        let got = printer().render_fun_def(&fx.sig("l", false), &body, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def l : Nat := Id.run for i in [0:10:1]i do x\n");
        // Ascribed bitvector literal joins.
        let mk_bit = |fx: &Fixture, b: &str| {
            AstExp::dummy(AstExpKind::Lit(AstLit::dummy(AstLitKind::Bin(fx.intern(b)))))
        };
        let eight = sail_ast::NExp::dummy(sail_ast::NExpKind::Constant("8".to_string()));
        let bits = AstTyp::dummy(AstTypKind::App {
            name: fx.name("bits"),
            args: vec![AstTypArg::dummy(AstTypArgKind::NExp(eight))],
        });
        let vec_lit = AstExp::dummy(AstExpKind::Typ {
            typ: bits,
            exp: Box::new(AstExp::dummy(AstExpKind::Vector(vec![mk_bit(&fx, "1"), mk_bit(&fx, "0")]))),
        });
        let got = printer().render_fun_def(&fx.sig("v", false), &vec_lit, &*fx.it.borrow(), &ExpCtx::pure()).unwrap();
        assert_eq!(got, "def v : Nat := (BitVec.join1 [1#1, 0#1])\n");
    }

    #[test]
    fn untranslatable_bodies_are_hard_errors() {
        let fx = Fixture::new();
        let ctx = ExpCtx::pure();
        let sig = fx.sig("f", false);
        // Loop combinator application.
        let loop_app = fx.app("while#", vec![fx.id("c"), fx.id("v"), fx.id("b")]);
        let err = printer().render_fun_def(&sig, &loop_app, &*fx.it.borrow(), &ctx).unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        // Bare vector literal (type-directed choice unavailable).
        let bare = AstExp::dummy(AstExpKind::Vector(vec![fx.num("1")]));
        let err = printer().render_fun_def(&sig, &bare, &*fx.it.borrow(), &ctx).unwrap_err();
        assert!(err.to_string().contains("E_typ"), "{err}");
        // `E_undef`.
        let undef = AstExp::dummy(AstExpKind::Undef);
        let err = printer().render_fun_def(&sig, &undef, &*fx.it.borrow(), &ctx).unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        // Multi-expression block in pure position.
        let block = AstExp::dummy(AstExpKind::Block(vec![fx.num("1"), fx.num("2")]));
        let err = printer().render_fun_def(&sig, &block, &*fx.it.borrow(), &ctx).unwrap_err();
        assert!(err.to_string().contains("E_block"), "{err}");
        // Real literal (needs the Real support module).
        let real = AstLit::dummy(AstLitKind::Real(fx.intern("1.5")));
        let real_exp = AstExp::dummy(AstExpKind::Lit(real));
        let err = printer().render_fun_def(&sig, &real_exp, &*fx.it.borrow(), &ctx).unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        // No error message smuggles a proof hole.
        for e in [
            printer().render_fun_def(&sig, &loop_app, &*fx.it.borrow(), &ctx).unwrap_err(),
            printer().render_fun_def(&sig, &bare, &*fx.it.borrow(), &ctx).unwrap_err(),
            printer().render_fun_def(&sig, &undef, &*fx.it.borrow(), &ctx).unwrap_err(),
        ] {
            assert!(!e.to_string().contains("sorry"), "{e}");
        }
    }

    #[test]
    fn when_in_plain_match_is_hard_error() {
        let fx = Fixture::new();
        let arm = AstPExp { id: NodeId::DUMMY, span: crate::Span::DUMMY, kind: AstPExpKind::PatWhen {
            pat: fx.pat_id("x"), guard: fx.id("c"), body: fx.num("1"),
        } };
        let mat = AstExp::dummy(AstExpKind::Match { scrutinee: Box::new(fx.id("v")), arms: vec![arm] });
        let err = printer()
            .render_fun_def(&fx.sig("w", false), &mat, &*fx.it.borrow(), &ExpCtx::pure())
            .unwrap_err();
        assert!(err.to_string().contains("when-clause"), "{err}");
    }

    #[test]
    fn lit_widths_follow_ocaml_doc_lit() {
        let fx = Fixture::new();
        let hex = AstLit::dummy(AstLitKind::Hex(fx.intern("ff")));
        let lowered = crate::conv::lit_from_ast(&hex, &*fx.it.borrow()).unwrap();
        let mut buf = String::new();
        printer().print_lit(&mut buf, &lowered).unwrap();
        assert_eq!(buf, "0xFF#8");
    }
}
