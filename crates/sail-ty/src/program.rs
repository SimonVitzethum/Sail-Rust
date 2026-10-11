#![forbid(unsafe_code)]
//! Canonical AST typing environment. Declaration histories provide immutable
//! source-position views without copying the whole environment per worker.
//! Function bodies stay in the driver's out-of-core store.

use crate::error::{TypeError, TypeErrorKind, TypeResult};
use sail_ast::*;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Scheme {
    pub typschm: TypSchm,
    pub interner: Arc<SimpleInterner>,
}
#[derive(Clone, Debug)]
struct Binding<T> {
    position: usize,
    value: T,
}
#[derive(Clone, Debug)]
struct Constructor {
    args: Vec<Kind>,
    result: Kind,
    requires_proof: bool,
}
#[derive(Clone, Debug)]
struct Value {
    scheme: Arc<Scheme>,
    enumeration_member: bool,
}

/// Canonical declaration tables, frozen before parallel body checking. Only
/// signatures/type definitions belong here, never function bodies.
#[derive(Default)]
pub struct Program {
    constructors: BTreeMap<String, Vec<Binding<Constructor>>>,
    values: BTreeMap<String, Vec<Binding<Value>>>,
    registers: BTreeMap<String, Vec<Binding<Arc<Scheme>>>>,
}

fn at<T>(history: &[Binding<T>], position: usize) -> Option<&T> {
    let index = history.partition_point(|binding| binding.position <= position);
    index.checked_sub(1).map(|index| &history[index].value)
}
fn text(it: &impl Interner, symbol: Symbol, span: Span) -> TypeResult<&str> {
    it.resolve(symbol)
        .ok_or_else(|| TypeError::unbound_var(span, format!("symbol {symbol}")))
}
fn failure(span: Span, message: impl Into<String>) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::QuantError(message.into()),
    }
}
fn constructor(args: &[Kind], result: Kind) -> Constructor {
    Constructor {
        args: args.to_vec(),
        result,
        requires_proof: false,
    }
}
fn builtin(name: &str) -> Option<Constructor> {
    use Kind::{Bool, Int, Type};
    Some(match name {
        "range" => constructor(&[Int, Int], Type),
        "atom" | "implicit" | "bitvector" | "itself" => constructor(&[Int], Type),
        "vector" => constructor(&[Int, Type], Type),
        "register" | "list" => constructor(&[Type], Type),
        "atom_bool" => constructor(&[Bool], Type),
        "unit"
        | "int"
        | "nat"
        | "bool"
        | "real"
        | "string"
        | "string_literal"
        | "float16"
        | "float32"
        | "float64"
        | "float128"
        | "float_rounding_mode" => constructor(&[], Type),
        "abs" => constructor(&[Int], Int),
        "mod" | "div" => constructor(&[Int, Int], Int),
        // Core NC_app("not", [A_bool _]) is emitted by initial_check rather
        // than declared in Sail. See type_check.ml's rewrite_nc rule.
        "not" => constructor(&[Bool], Bool),
        _ => return None,
    })
}
fn quant_args(quant: &[QuantItem]) -> Vec<Kind> {
    quant
        .iter()
        .filter_map(|q| match q {
            QuantItem::Id(k) => Some(k.kind),
            QuantItem::Constraint(_) => None,
        })
        .collect()
}

impl Program {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check one loaded definition against its immutable source-position view.
    /// This method owns no function body and can run on bounded worker pools.
    pub fn check_definition(
        &self,
        def: &Def,
        it: Arc<SimpleInterner>,
        position: usize,
    ) -> TypeResult<usize> {
        crate::canonical_body::check(self, def, it, position)
    }

    /// Check and retain a declaration in source order. Repeated val specs
    /// create history entries: an earlier body keeps its earlier signature.
    pub fn declare(
        &mut self,
        def: &Def,
        it: Arc<SimpleInterner>,
        position: usize,
    ) -> TypeResult<()> {
        match &def.kind {
            DefKind::Type(typ) => {
                let (name, quant, result) = match &typ.kind {
                    TypeDefKind::Abbrev { name, quant, arg } => {
                        (name, quant.as_slice(), arg_kind(arg))
                    }
                    TypeDefKind::Record { name, quant, .. }
                    | TypeDefKind::Variant { name, quant, .. } => {
                        (name, quant.as_slice(), Kind::Type)
                    }
                    TypeDefKind::Enum { name, .. } | TypeDefKind::Bitfield { name, .. } => {
                        (name, &[][..], Kind::Type)
                    }
                    TypeDefKind::Abstract { name, kind } => (name, &[][..], *kind),
                };
                let name = text(it.as_ref(), name.symbol, typ.span)?.to_owned();
                if builtin(&name).is_some() || self.constructors.contains_key(&name) {
                    return Err(failure(
                        typ.span,
                        format!("type constructor '{name}' is already defined"),
                    ));
                }
                let signature = Constructor {
                    args: quant_args(quant),
                    result,
                    requires_proof: quant.iter().any(|item| matches!(item, QuantItem::Constraint(c) if !matches!(c.kind, ConstraintKind::True))),
                };
                // OCaml's forbid_recursive_types checks record/union payloads
                // before inserting their constructor into the environment.
                let mut checker = WellFormed::new(self, &it, position);
                checker.quant(quant)?;
                match &typ.kind {
                    TypeDefKind::Abbrev { arg, .. } => checker.run(vec![Task::Arg(arg, result)])?,
                    TypeDefKind::Record { fields, .. } => {
                        checker.run(fields.iter().map(|(_, typ)| Task::Typ(typ)).collect())?
                    }
                    TypeDefKind::Variant { ctors, .. } => {
                        checker.run(ctors.iter().map(|ctor| Task::Typ(&ctor.typ)).collect())?
                    }
                    TypeDefKind::Bitfield { typ, fields, .. } => {
                        let mut tasks = vec![Task::Typ(typ)];
                        tasks.extend(fields.iter().map(|(_, range)| Task::Range(range)));
                        checker.run(tasks)?;
                    }
                    TypeDefKind::Enum { .. } | TypeDefKind::Abstract { .. } => {}
                }
                let mut members = Vec::new();
                match &typ.kind {
                    TypeDefKind::Enum {
                        name,
                        members: names,
                        ..
                    } => {
                        for member in names {
                            members.push((
                                text(it.as_ref(), member.symbol, member.span)?.to_owned(),
                                member.span,
                                Value {
                                    scheme: Arc::new(Scheme {
                                        typschm: TypSchm {
                                            quant: vec![],
                                            typ: Node::dummy(TypKind::Id(name.clone())),
                                        },
                                        interner: it.clone(),
                                    }),
                                    enumeration_member: true,
                                },
                            ));
                        }
                    }
                    TypeDefKind::Variant {
                        name, quant, ctors, ..
                    } => {
                        let args: Vec<_> = quant
                            .iter()
                            .filter_map(|item| match item {
                                QuantItem::Constraint(_) => None,
                                QuantItem::Id(binder) => Some(Node::dummy(match binder.kind {
                                    Kind::Type => {
                                        TypArgKind::Typ(Node::dummy(TypKind::Var(binder.kid)))
                                    }
                                    Kind::Int => {
                                        TypArgKind::NExp(Node::dummy(NExpKind::Var(binder.kid)))
                                    }
                                    Kind::Bool => TypArgKind::Bool(Box::new(Node::dummy(
                                        ConstraintKind::Var(binder.kid),
                                    ))),
                                })),
                            })
                            .collect();
                        let ret = Node::dummy(if args.is_empty() {
                            TypKind::Id(name.clone())
                        } else {
                            TypKind::App {
                                name: name.clone(),
                                args,
                            }
                        });
                        for ctor in ctors {
                            let scheme = Scheme {
                                typschm: TypSchm {
                                    quant: quant.clone(),
                                    typ: Node::dummy(TypKind::Fn {
                                        args: vec![ctor.typ.clone()],
                                        ret: Box::new(ret.clone()),
                                    }),
                                },
                                interner: it.clone(),
                            };
                            members.push((
                                text(it.as_ref(), ctor.ctor.symbol, ctor.span)?.to_owned(),
                                ctor.span,
                                Value {
                                    scheme: Arc::new(scheme),
                                    enumeration_member: false,
                                },
                            ));
                        }
                    }
                    _ => {}
                }
                let mut seen = std::collections::BTreeSet::new();
                for (name, span, _) in &members {
                    if !seen.insert(name)
                        || self.values.contains_key(name)
                        || self.registers.contains_key(name)
                    {
                        return Err(failure(
                            *span,
                            format!("constructor/member '{name}' is already bound"),
                        ));
                    }
                }
                self.constructors.entry(name).or_default().push(Binding {
                    position,
                    value: signature,
                });
                for (name, _, value) in members {
                    self.values
                        .entry(name)
                        .or_default()
                        .push(Binding { position, value });
                }
                Ok(())
            }
            DefKind::Val(val) => {
                self.check_scheme(&val.typschm, &it, position)?;
                if !matches!(
                    val.typschm.typ.kind,
                    TypKind::Fn { .. } | TypKind::Bidir { .. }
                ) {
                    return Err(TypeError {
                        span: val.span,
                        kind: TypeErrorKind::Unsupported("val signature synonym expansion".into()),
                    });
                }
                let name = text(it.as_ref(), val.name.symbol, val.span)?.to_owned();
                self.values.entry(name).or_default().push(Binding {
                    position,
                    value: Value {
                        enumeration_member: false,
                        scheme: Arc::new(Scheme {
                            typschm: val.typschm.clone(),
                            interner: it,
                        }),
                    },
                });
                Ok(())
            }
            DefKind::Register(reg) => {
                self.check_scheme(
                    &TypSchm {
                        quant: vec![],
                        typ: reg.typ.clone(),
                    },
                    &it,
                    position,
                )?;
                let name = text(it.as_ref(), reg.name.symbol, reg.span)?.to_owned();
                self.registers.entry(name).or_default().push(Binding {
                    position,
                    value: Arc::new(Scheme {
                        typschm: TypSchm {
                            quant: vec![],
                            typ: reg.typ.clone(),
                        },
                        interner: it,
                    }),
                });
                Ok(())
            }
            DefKind::Constraint(constraint) => {
                WellFormed::new(self, &it, position).run(vec![Task::Constraint(constraint)])
            }
            // Bodies and definitions requiring inference are checked by the
            // body pass; accepting their header does not accept the body.
            _ => Ok(()),
        }
    }

    pub fn check_scheme(
        &self,
        scheme: &TypSchm,
        it: &SimpleInterner,
        position: usize,
    ) -> TypeResult<()> {
        let mut checker = WellFormed::new(self, it, position);
        checker.quant(&scheme.quant)?;
        checker.run(vec![Task::Typ(&scheme.typ)])
    }

    /// Validate an expression/pattern annotation in its function's quantified
    /// scope. Interners are definition-local, so outer binders match by name.
    pub(crate) fn check_annotation(
        &self,
        typ: &Typ,
        it: &SimpleInterner,
        position: usize,
        scope: Option<&Scheme>,
    ) -> TypeResult<()> {
        let mut checker = WellFormed::new(self, it, position);
        if let Some(scope) = scope {
            for item in &scope.typschm.quant {
                if let QuantItem::Id(binder) = item {
                    checker.outer.insert(
                        text(scope.interner.as_ref(), binder.kid, typ.span)?.to_owned(),
                        binder.kind,
                    );
                }
            }
        }
        checker.run(vec![Task::Typ(typ)])
    }

    /// Source-position lookup, safe to call concurrently after declarations
    /// are frozen. A worker never sees a later replacement or declaration.
    #[must_use]
    pub fn value(&self, name: &str, position: usize) -> Option<&Arc<Scheme>> {
        self.values
            .get(name)
            .and_then(|history| at(history, position))
            .map(|value| &value.scheme)
    }
    pub(crate) fn enumeration_member(&self, name: &str, position: usize) -> Option<&Arc<Scheme>> {
        self.values
            .get(name)
            .and_then(|history| at(history, position))
            .filter(|value| value.enumeration_member)
            .map(|value| &value.scheme)
    }
    #[must_use]
    pub fn register(&self, name: &str, position: usize) -> Option<&Arc<Scheme>> {
        self.registers
            .get(name)
            .and_then(|history| at(history, position))
    }

    fn signature(&self, name: &str, position: usize) -> Option<Constructor> {
        self.constructors
            .get(name)
            .and_then(|history| at(history, position))
            .cloned()
            .or_else(|| builtin(name))
    }
}

fn arg_kind(arg: &TypArg) -> Kind {
    match arg.kind {
        TypArgKind::NExp(_) => Kind::Int,
        TypArgKind::Typ(_) => Kind::Type,
        TypArgKind::Bool(_) => Kind::Bool,
    }
}
enum Task<'a> {
    Typ(&'a Typ),
    Arg(&'a TypArg, Kind),
    NExp(&'a NExp),
    Constraint(&'a Constraint),
    Range(&'a IndexRange),
    Restore(Vec<(Symbol, Option<Kind>)>),
}
struct WellFormed<'a> {
    program: &'a Program,
    it: &'a SimpleInterner,
    position: usize,
    vars: BTreeMap<Symbol, Kind>,
    outer: BTreeMap<String, Kind>,
    in_existential: bool,
}
impl<'a> WellFormed<'a> {
    fn new(program: &'a Program, it: &'a SimpleInterner, position: usize) -> Self {
        Self {
            program,
            it,
            position,
            vars: BTreeMap::new(),
            outer: BTreeMap::new(),
            in_existential: false,
        }
    }
    fn quant(&mut self, quant: &'a [QuantItem]) -> TypeResult<()> {
        for item in quant {
            if let QuantItem::Id(binder) = item {
                if self.vars.insert(binder.kid, binder.kind).is_some() {
                    return Err(failure(
                        Span::DUMMY,
                        format!(
                            "duplicate quantified variable {}",
                            text(self.it, binder.kid, Span::DUMMY)?
                        ),
                    ));
                }
            }
        }
        self.run(
            quant
                .iter()
                .filter_map(|item| match item {
                    QuantItem::Id(_) => None,
                    QuantItem::Constraint(c) => Some(Task::Constraint(c)),
                })
                .collect(),
        )
    }
    fn variable(&self, symbol: Symbol, expected: Kind, span: Span) -> TypeResult<()> {
        let name = text(self.it, symbol, span)?;
        match self.vars.get(&symbol).or_else(|| self.outer.get(name)) {
            Some(found) if *found == expected => Ok(()),
            Some(found) => Err(failure(
                span,
                format!("kind mismatch for {name}: expected {expected:?}, found {found:?}"),
            )),
            None => Err(TypeError::unbound_var(span, name.to_owned())),
        }
    }
    fn application(&self, name: &Name, arity: usize, result: Kind) -> TypeResult<Constructor> {
        let name_text = text(self.it, name.symbol, name.span)?;
        let signature = self
            .program
            .signature(name_text, self.position)
            .ok_or_else(|| TypeError::unbound_var(name.span, name_text.to_owned()))?;
        if signature.result != result {
            return Err(failure(
                name.span,
                format!(
                    "constructor {name_text} returns {:?}, expected {result:?}",
                    signature.result
                ),
            ));
        }
        if signature.args.len() != arity {
            return Err(TypeError {
                span: name.span,
                kind: TypeErrorKind::ArityMismatch {
                    expected: signature.args.len(),
                    found: arity,
                },
            });
        }
        if signature.requires_proof {
            return Err(TypeError {
                span: name.span,
                kind: TypeErrorKind::Unsupported(format!(
                    "constraint discharge for type constructor '{name_text}'"
                )),
            });
        }
        Ok(signature)
    }
    fn run(&mut self, mut work: Vec<Task<'a>>) -> TypeResult<()> {
        while let Some(task) = work.pop() {
            match task {
                Task::Range(range) => match &range.kind {
                    IndexRangeKind::Single(n) => work.push(Task::NExp(n)),
                    IndexRangeKind::Range { lo, hi } => {
                        work.push(Task::NExp(hi));
                        work.push(Task::NExp(lo));
                    }
                    IndexRangeKind::Concat(a, b) => {
                        work.push(Task::Range(b));
                        work.push(Task::Range(a));
                    }
                },
                Task::Typ(typ) => match &typ.kind {
                    TypKind::Unknown => {
                        return Err(failure(typ.span, "unresolved type placeholder"))
                    }
                    TypKind::Id(name) => {
                        self.application(name, 0, Kind::Type)?;
                    }
                    TypKind::Var(symbol) => self.variable(*symbol, Kind::Type, typ.span)?,
                    TypKind::Fn { args, ret } => {
                        work.push(Task::Typ(ret));
                        work.extend(args.iter().map(Task::Typ));
                    }
                    TypKind::Bidir { lhs, rhs } => {
                        work.push(Task::Typ(rhs));
                        work.push(Task::Typ(lhs));
                    }
                    TypKind::Tuple(items) => work.extend(items.iter().map(Task::Typ)),
                    TypKind::App { name, args } => {
                        let signature = self.application(name, args.len(), Kind::Type)?;
                        work.extend(
                            args.iter()
                                .zip(signature.args)
                                .map(|(arg, kind)| Task::Arg(arg, kind)),
                        );
                    }
                    TypKind::Exist {
                        binders,
                        constraint,
                        typ,
                    } => {
                        if binders.is_empty() || self.in_existential {
                            return Err(failure(
                                typ.span,
                                "existentials require binders and cannot nest",
                            ));
                        }
                        let mut names = std::collections::BTreeSet::new();
                        if binders.iter().any(|binder| !names.insert(binder.kid)) {
                            return Err(failure(typ.span, "duplicate existential binder"));
                        }
                        self.in_existential = true;
                        let restored = binders
                            .iter()
                            .map(|binder| (binder.kid, self.vars.insert(binder.kid, binder.kind)))
                            .collect();
                        work.push(Task::Restore(restored));
                        work.push(Task::Typ(typ));
                        work.push(Task::Constraint(constraint));
                    }
                },
                Task::Arg(arg, expected) => {
                    let found = arg_kind(arg);
                    if found != expected {
                        return Err(failure(arg.span, format!("type argument kind mismatch: expected {expected:?}, found {found:?}")));
                    }
                    work.push(match &arg.kind {
                        TypArgKind::Typ(t) => Task::Typ(t),
                        TypArgKind::NExp(n) => Task::NExp(n),
                        TypArgKind::Bool(c) => Task::Constraint(c),
                    });
                }
                Task::NExp(n) => match &n.kind {
                    NExpKind::Constant(_) => {}
                    NExpKind::Var(symbol) => self.variable(*symbol, Kind::Int, n.span)?,
                    NExpKind::Id(name) => {
                        self.application(name, 0, Kind::Int)?;
                    }
                    NExpKind::App { func, args } => {
                        let signature = self.application(func, args.len(), Kind::Int)?;
                        if signature.args.iter().any(|kind| *kind != Kind::Int) {
                            return Err(failure(
                                n.span,
                                "numeric application argument kind mismatch",
                            ));
                        }
                        work.extend(args.iter().map(Task::NExp));
                    }
                    NExpKind::Times(a, b) | NExpKind::Sum(a, b) | NExpKind::Minus(a, b) => {
                        work.push(Task::NExp(b));
                        work.push(Task::NExp(a));
                    }
                    NExpKind::Exp(n) | NExpKind::Neg(n) => work.push(Task::NExp(n)),
                    NExpKind::If {
                        cond,
                        then_branch,
                        else_branch,
                    } => {
                        work.push(Task::NExp(else_branch));
                        work.push(Task::NExp(then_branch));
                        work.push(Task::Constraint(cond));
                    }
                },
                Task::Constraint(c) => match &c.kind {
                    ConstraintKind::True | ConstraintKind::False => {}
                    ConstraintKind::Id(name) => {
                        self.application(name, 0, Kind::Bool)?;
                    }
                    ConstraintKind::Var(symbol) => self.variable(*symbol, Kind::Bool, c.span)?,
                    ConstraintKind::Equal(a, b) | ConstraintKind::NotEqual(a, b) => {
                        let kind = arg_kind(a);
                        work.push(Task::Arg(b, kind));
                        work.push(Task::Arg(a, kind));
                    }
                    ConstraintKind::Ge(a, b)
                    | ConstraintKind::Gt(a, b)
                    | ConstraintKind::Le(a, b)
                    | ConstraintKind::Lt(a, b) => {
                        work.push(Task::NExp(b));
                        work.push(Task::NExp(a));
                    }
                    ConstraintKind::Set { nexp, .. } => work.push(Task::NExp(nexp)),
                    ConstraintKind::And(a, b) | ConstraintKind::Or(a, b) => {
                        work.push(Task::Constraint(b));
                        work.push(Task::Constraint(a));
                    }
                    ConstraintKind::Not(c) => work.push(Task::Constraint(c)),
                    ConstraintKind::App { func, args } => {
                        let signature = self.application(func, args.len(), Kind::Bool)?;
                        work.extend(
                            args.iter()
                                .zip(signature.args)
                                .map(|(arg, kind)| Task::Arg(arg, kind)),
                        );
                    }
                },
                Task::Restore(vars) => {
                    self.in_existential = false;
                    for (symbol, previous) in vars.into_iter().rev() {
                        if let Some(kind) = previous {
                            self.vars.insert(symbol, kind);
                        } else {
                            self.vars.remove(&symbol);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
