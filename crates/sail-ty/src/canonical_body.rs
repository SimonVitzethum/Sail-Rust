#![forbid(unsafe_code)]
//! Bidirectional checking over the canonical AST using explicit work stacks.
//! Unsupported rules fail; no placeholder or unproved obligation is accepted.

use crate::{
    error::{TypeError, TypeErrorKind, TypeResult},
    program::{Program, Scheme},
};
use sail_ast::*;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

#[derive(Clone)]
enum Step {
    Argument(usize),
    Result,
    Tuple(usize),
    TypeArgument(usize),
}
#[derive(Clone)]
struct Type {
    root: Arc<Scheme>,
    path: Vec<Step>,
    /// Rigid binder identity survives annotations with a different interner.
    scope: Arc<Scheme>,
    substitutions: Arc<BTreeMap<Symbol, Type>>,
}
impl Type {
    fn root(root: Arc<Scheme>) -> Self {
        Self {
            scope: root.clone(),
            root,
            path: vec![],
            substitutions: Arc::default(),
        }
    }
    fn node(&self) -> &Typ {
        let mut typ = &self.root.typschm.typ;
        for step in &self.path {
            typ = match (step, &typ.kind) {
                (Step::Argument(i), TypKind::Fn { args, .. }) => &args[*i],
                (Step::Result, TypKind::Fn { ret, .. }) => ret,
                (Step::Tuple(i), TypKind::Tuple(items)) => &items[*i],
                (Step::TypeArgument(i), TypKind::App { args, .. }) => {
                    let TypArgKind::Typ(typ) = &args[*i].kind else {
                        unreachable!()
                    };
                    typ
                }
                _ => unreachable!("checked type path"),
            };
        }
        typ
    }
    fn child(&self, step: Step) -> Self {
        let mut child = self.clone();
        child.path.push(step);
        child
    }
    fn resolved(&self) -> Self {
        let mut typ = self.clone();
        while let TypKind::Var(symbol) = typ.node().kind {
            let Some(bound) = typ.substitutions.get(&symbol) else {
                break;
            };
            typ = bound.clone();
        }
        typ
    }
    fn owned(typ: Typ, it: Arc<SimpleInterner>) -> Self {
        Self::root(Arc::new(Scheme {
            typschm: TypSchm { quant: vec![], typ },
            interner: it,
        }))
    }
    fn name(&self) -> Option<&str> {
        match &self.node().kind {
            TypKind::Id(name) | TypKind::App { name, .. } => {
                self.root.interner.resolve(name.symbol)
            }
            _ => None,
        }
    }
    fn label(&self) -> String {
        self.name()
            .map(str::to_owned)
            .unwrap_or_else(|| match &self.node().kind {
                TypKind::Var(symbol) => self
                    .root
                    .interner
                    .resolve(*symbol)
                    .unwrap_or("<unbound>")
                    .into(),
                TypKind::Tuple(items) => format!("tuple of {} elements", items.len()),
                TypKind::Fn { .. } => "function".into(),
                TypKind::Bidir { .. } => "mapping".into(),
                TypKind::Exist { .. } => "existential type".into(),
                TypKind::Unknown => "unknown type".into(),
                TypKind::Id(_) | TypKind::App { .. } => "unbound type name".into(),
            })
    }
}
#[derive(Default)]
struct Scope {
    locals: BTreeMap<String, (Type, bool)>,
    parent: Option<Arc<Scope>>,
}
impl Drop for Scope {
    fn drop(&mut self) {
        let mut parent = self.parent.take();
        while let Some(scope) = parent {
            match Arc::try_unwrap(scope) {
                Ok(mut scope) => parent = scope.parent.take(),
                Err(_) => break,
            }
        }
    }
}
impl Scope {
    fn get(&self, name: &str) -> Option<&(Type, bool)> {
        let mut scope = Some(self);
        while let Some(current) = scope {
            if let Some(typ) = current.locals.get(name) {
                return Some(typ);
            }
            scope = current.parent.as_deref();
        }
        None
    }
}
fn unsupported(span: Span, message: impl Into<String>) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::Unsupported(message.into()),
    }
}
fn arity(span: Span, expected: usize, found: usize) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::ArityMismatch { expected, found },
    }
}
fn mismatch(span: Span, expected: &Type, found: &Type) -> TypeError {
    TypeError::mismatch(span, expected.label(), found.label())
}

struct Checker<'a> {
    program: &'a Program,
    it: Arc<SimpleInterner>,
    position: usize,
    builtins: BTreeMap<&'static str, Type>,
    expressions: usize,
    context: Option<Arc<Scheme>>,
    /// Per-definition synthesis cache; pointers are identity keys only and
    /// are never dereferenced. The loaded AST outlives this checker.
    inferred: HashMap<*const Exp, Type>,
}
struct Task<'a> {
    exp: &'a Exp,
    expected: Type,
    scope: Arc<Scope>,
    returning: Type,
}

pub(crate) fn check(
    program: &Program,
    def: &Def,
    it: Arc<SimpleInterner>,
    position: usize,
) -> TypeResult<usize> {
    let mut checker = Checker {
        program,
        it,
        position,
        builtins: BTreeMap::new(),
        expressions: 0,
        context: None,
        inferred: HashMap::new(),
    };
    match &def.kind {
        DefKind::Fundef { rec, annot, funcls } => {
            if !matches!(annot, TAnnotOpt::None) {
                return Err(unsupported(
                    def.span,
                    "function signature annotation elaboration",
                ));
            }
            if matches!(rec, RecOpt::Measure { .. }) {
                return Err(unsupported(def.span, "termination-measure typing"));
            }
            for funcl in funcls {
                checker.function(funcl)?;
            }
        }
        DefKind::Impl(_)
        | DefKind::Mapdef { .. }
        | DefKind::Scattered(_)
        | DefKind::Outcome { .. }
        | DefKind::Instantiation { .. }
        | DefKind::Measure { .. }
        | DefKind::LoopMeasures { .. }
        | DefKind::InternalMutrec(_) => {
            return Err(unsupported(
                def.span,
                "mapping, scattered, outcome or measure definition",
            ))
        }
        DefKind::Let { pat, exp } => {
            let scope = Arc::new(Scope::default());
            let typ = checker.infer(exp, &scope)?;
            let mut locals = Scope::default();
            checker.pattern(pat, typ.clone(), false, &mut locals)?;
            checker.run(vec![Task {
                exp,
                expected: typ.clone(),
                scope,
                returning: typ,
            }])?;
        }
        DefKind::Register(register) => {
            if let Some(exp) = &register.init {
                let typ = Type::owned(register.typ.clone(), checker.it.clone());
                checker.run(vec![Task {
                    exp,
                    expected: typ.clone(),
                    scope: Arc::new(Scope::default()),
                    returning: typ,
                }])?;
            }
        }
        DefKind::Type(_)
        | DefKind::Val(_)
        | DefKind::Constraint(_)
        | DefKind::Fixity { .. }
        | DefKind::Overload { .. }
        | DefKind::Default(_)
        | DefKind::Pragma { .. } => {}
    }
    Ok(checker.expressions)
}

impl Checker<'_> {
    fn annotation(&self, typ: &Typ) -> TypeResult<Type> {
        self.program
            .check_annotation(typ, &self.it, self.position, self.context.as_deref())?;
        let mut typ = Type::owned(typ.clone(), self.it.clone());
        if let Some(scope) = &self.context {
            typ.scope = scope.clone();
        }
        Ok(typ)
    }
    fn builtin(&mut self, name: &'static str) -> Type {
        self.builtins
            .entry(name)
            .or_insert_with(|| {
                let mut it = SimpleInterner::new();
                let symbol = it.intern(name);
                Type::owned(Node::dummy(TypKind::Id(Name::dummy(symbol))), Arc::new(it))
            })
            .clone()
    }
    fn name(&self, name: &Name) -> TypeResult<&str> {
        self.it
            .resolve(name.symbol)
            .ok_or_else(|| TypeError::unbound_var(name.span, format!("symbol {}", name.symbol)))
    }
    fn id(&self, name: &Name, scope: &Scope) -> TypeResult<Type> {
        let name_text = self.name(name)?;
        if let Some((typ, _)) = scope.get(name_text) {
            return Ok(typ.clone());
        }
        self.program
            .register(name_text, self.position)
            .or_else(|| self.program.value(name_text, self.position))
            .cloned()
            .map(Type::root)
            .ok_or_else(|| TypeError::unbound_var(name.span, name_text.to_owned()))
    }
    fn literal(&mut self, lit: &Lit) -> TypeResult<Type> {
        Ok(match lit.kind {
            LitKind::Unit => self.builtin("unit"),
            LitKind::True | LitKind::False => self.refined(
                "atom_bool",
                TypArgKind::Bool(Box::new(Node::dummy(
                    if matches!(lit.kind, LitKind::True) {
                        ConstraintKind::True
                    } else {
                        ConstraintKind::False
                    },
                ))),
            ),
            LitKind::Num(symbol) => {
                let value = self.it.resolve(symbol).ok_or_else(|| {
                    TypeError::unbound_var(lit.span, "numeric literal symbol".into())
                })?;
                self.refined(
                    "atom",
                    TypArgKind::NExp(Node::dummy(NExpKind::Constant(value.to_owned()))),
                )
            }
            LitKind::Real(_) => self.builtin("real"),
            LitKind::String(_) => self.builtin("string"),
            LitKind::Bin(symbol) | LitKind::Hex(symbol) => {
                let value = self
                    .it
                    .resolve(symbol)
                    .ok_or_else(|| TypeError::unbound_var(lit.span, "bit literal symbol".into()))?;
                let digits = value.bytes().filter(|byte| *byte != b'_').count();
                let width = digits
                    .checked_mul(if matches!(lit.kind, LitKind::Hex(_)) {
                        4
                    } else {
                        1
                    })
                    .ok_or_else(|| unsupported(lit.span, "literal width overflow"))?;
                self.refined(
                    "bitvector",
                    TypArgKind::NExp(Node::dummy(NExpKind::Constant(width.to_string()))),
                )
            }
        })
    }
    fn refined(&self, name: &str, arg: TypArgKind) -> Type {
        let mut it = SimpleInterner::new();
        let name = Name::dummy(it.intern(name));
        Type::owned(
            Node::dummy(TypKind::App {
                name,
                args: vec![Node::dummy(arg)],
            }),
            Arc::new(it),
        )
    }
    fn compatible(&self, expected: &Type, found: &Type, span: Span) -> TypeResult<()> {
        let mut pairs = vec![(expected.clone(), found.clone())];
        while let Some((expected, found)) = pairs.pop() {
            let expected = expected.resolved();
            let found = found.resolved();
            if matches!(
                (&expected.node().kind, expected.name(), found.name()),
                (TypKind::Id(_), Some("int"), Some("atom" | "range" | "nat"))
                    | (TypKind::Id(_), Some("bool"), Some("atom_bool"))
            ) {
                continue;
            }
            if let (TypKind::Id(_), Some(base), Some(refined)) =
                (&expected.node().kind, expected.name(), found.name())
            {
                if matches!(base, "unit" | "bool" | "int" | "real" | "string")
                    && matches!(refined, "atom" | "atom_bool" | "bitvector")
                {
                    return Err(mismatch(span, &expected, &found));
                }
            }
            match (&expected.node().kind, &found.node().kind) {
                (TypKind::Id(a), TypKind::Id(b)) => {
                    if expected.root.interner.resolve(a.symbol)
                        != found.root.interner.resolve(b.symbol)
                    {
                        return Err(mismatch(span, &expected, &found));
                    }
                }
                (TypKind::Var(a), TypKind::Var(b))
                    if Arc::ptr_eq(&expected.scope, &found.scope)
                        && expected.root.interner.resolve(*a)
                            == found.root.interner.resolve(*b) => {}
                (TypKind::Tuple(a), TypKind::Tuple(b)) => {
                    if a.len() != b.len() {
                        return Err(arity(span, a.len(), b.len()));
                    }
                    for i in 0..a.len() {
                        pairs.push((expected.child(Step::Tuple(i)), found.child(Step::Tuple(i))));
                    }
                }
                (TypKind::App { name: a, args: aa }, TypKind::App { name: b, args: ba })
                    if expected.root.interner.resolve(a.symbol)
                        == found.root.interner.resolve(b.symbol) =>
                {
                    if aa.len() != ba.len() {
                        return Err(arity(span, aa.len(), ba.len()));
                    }
                    for (i, (a, b)) in aa.iter().zip(ba).enumerate() {
                        match (&a.kind, &b.kind) {
                            (TypArgKind::Typ(_), TypArgKind::Typ(_)) => pairs.push((
                                expected.child(Step::TypeArgument(i)),
                                found.child(Step::TypeArgument(i)),
                            )),
                            _ if crate::canonical_eq::argument(
                                a,
                                b,
                                &expected.root.interner,
                                &found.root.interner,
                                Arc::ptr_eq(&expected.scope, &found.scope),
                            ) => {}
                            _ => {
                                return Err(TypeError::constraint_not_proved(
                                    span,
                                    "type argument equivalence requires dependent solving",
                                ))
                            }
                        }
                    }
                }
                (TypKind::Unknown, _) | (_, TypKind::Unknown) => {
                    return Err(unsupported(span, "unresolved type placeholder"))
                }
                (TypKind::App { .. } | TypKind::Exist { .. }, _)
                | (_, TypKind::App { .. } | TypKind::Exist { .. }) => return Err(unsupported(
                    span,
                    "type synonym expansion, refinement subsumption and dependent argument solving",
                )),
                _ => return Err(mismatch(span, &expected, &found)),
            }
        }
        Ok(())
    }
    fn pattern(
        &mut self,
        pat: &Pat,
        typ: Type,
        mutable: bool,
        scope: &mut Scope,
    ) -> TypeResult<()> {
        let mut work = vec![(pat, typ)];
        while let Some((pat, typ)) = work.pop() {
            let typ = typ.resolved();
            match &pat.kind {
                PatKind::Wild => {}
                PatKind::Id(name) => {
                    let name_text = self.name(name)?;
                    if let Some(value) = self.program.enumeration_member(name_text, self.position) {
                        self.compatible(&typ, &Type::root(value.clone()), pat.span)?;
                    } else {
                        if self.program.register(name_text, self.position).is_some() {
                            return Err(TypeError {
                                span: pat.span,
                                kind: TypeErrorKind::RegisterError(format!(
                                    "cannot shadow register '{name_text}' in a pattern"
                                )),
                            });
                        }
                        if scope
                            .locals
                            .insert(name_text.to_owned(), (typ, mutable))
                            .is_some()
                        {
                            return Err(unsupported(pat.span, "nonlinear pattern binding"));
                        }
                    }
                }
                PatKind::As { pat, id } => {
                    scope
                        .locals
                        .insert(self.name(id)?.to_owned(), (typ.clone(), mutable));
                    work.push((pat, typ));
                }
                PatKind::Typ {
                    typ: annotation,
                    pat,
                } => {
                    let annotated = self.annotation(annotation)?;
                    self.compatible(&typ, &annotated, pat.span)?;
                    work.push((pat, annotated));
                }
                PatKind::Tuple(items) => {
                    let TypKind::Tuple(types) = &typ.node().kind else {
                        return Err(unsupported(
                            pat.span,
                            "tuple pattern against a type synonym",
                        ));
                    };
                    if items.len() != types.len() {
                        return Err(arity(pat.span, types.len(), items.len()));
                    }
                    for (i, pat) in items.iter().enumerate().rev() {
                        work.push((pat, typ.child(Step::Tuple(i))));
                    }
                }
                PatKind::Lit(lit) => {
                    let actual = self.literal(lit)?;
                    self.compatible(&typ, &actual, pat.span)?;
                }
                _ => {
                    return Err(unsupported(
                        pat.span,
                        "constructor, vector, list, disjunctive or dependent pattern binding",
                    ))
                }
            }
        }
        Ok(())
    }
    fn function(&mut self, funcl: &Funcl) -> TypeResult<()> {
        let name = self.name(&funcl.name)?;
        let scheme = self.program.value(name, self.position).cloned()
            .ok_or_else(|| unsupported(funcl.span, format!("cannot infer type of unannotated function '{name}' without a val specification")))?;
        let typ = Type::root(scheme);
        self.context = Some(typ.root.clone());
        let TypKind::Fn { args, .. } = &typ.node().kind else {
            return Err(unsupported(
                funcl.span,
                "function has a non-function signature",
            ));
        };
        let (pat, guard, body) = match &funcl.arm.kind {
            PExpKind::PatExp { pat, body } => (pat, None, body),
            PExpKind::PatWhen { pat, guard, body } => (pat, Some(guard), body),
        };
        let mut scope = Scope::default();
        if args.len() == 1 {
            self.pattern(pat, typ.child(Step::Argument(0)), false, &mut scope)?;
        } else {
            let PatKind::Tuple(items) = &pat.kind else {
                return Err(arity(pat.span, args.len(), 1));
            };
            if args.len() != items.len() {
                return Err(arity(pat.span, args.len(), items.len()));
            }
            for (index, pat) in items.iter().enumerate() {
                self.pattern(pat, typ.child(Step::Argument(index)), false, &mut scope)?;
            }
        }
        let result = typ.child(Step::Result);
        let scope = Arc::new(scope);
        let mut tasks = vec![Task {
            exp: body,
            expected: result.clone(),
            scope: scope.clone(),
            returning: result.clone(),
        }];
        if let Some(guard) = guard {
            tasks.push(Task {
                exp: guard,
                expected: self.builtin("bool"),
                scope,
                returning: result,
            });
        }
        self.run(tasks)
    }
    fn infer(&mut self, exp: &Exp, scope: &Scope) -> TypeResult<Type> {
        enum Synthesis<'a> {
            Enter(&'a Exp),
            Call(&'a Exp, Type, usize),
        }
        let mut work = vec![Synthesis::Enter(exp)];
        let mut results = Vec::new();
        while let Some(task) = work.pop() {
            match task {
                Synthesis::Enter(exp) => {
                    if let Some(typ) = self.inferred.get(&(exp as *const Exp)) {
                        results.push(typ.clone());
                        continue;
                    }
                    results.push(match &exp.kind {
                        ExpKind::Lit(lit) => self.literal(lit)?,
                        ExpKind::Id(name) => self.id(name, scope)?,
                        ExpKind::Typ { typ, .. } => self.annotation(typ)?,
                        ExpKind::App { func, args } => {
                            let callee = self.id(func, scope)?.resolved();
                            let TypKind::Fn { args: formal, .. } = &callee.node().kind else {
                                return Err(unsupported(exp.span, "application of non-function"));
                            };
                            if args.len() != formal.len() {
                                return Err(arity(exp.span, formal.len(), args.len()));
                            }
                            if callee.root.typschm.quant.is_empty() {
                                callee.child(Step::Result).resolved()
                            } else {
                                work.push(Synthesis::Call(exp, callee, args.len()));
                                work.extend(args.iter().rev().map(Synthesis::Enter));
                                continue;
                            }
                        }
                        _ => {
                            return Err(unsupported(
                                exp.span,
                                "synthesis for this expression requires further elaboration",
                            ))
                        }
                    });
                }
                Synthesis::Call(exp, callee, count) => {
                    let actual = results.split_off(results.len() - count);
                    let callee = self.instantiate(callee, &actual, None, exp.span)?;
                    let result = callee.child(Step::Result).resolved();
                    self.inferred.insert(exp as *const Exp, result.clone());
                    results.push(result);
                }
            }
        }
        Ok(results.pop().expect("synthesized root"))
    }

    fn instantiate(
        &self,
        mut callee: Type,
        actual: &[Type],
        expected: Option<&Type>,
        span: Span,
    ) -> TypeResult<Type> {
        let mut quantified = BTreeSet::new();
        for item in &callee.root.typschm.quant {
            match item {
                QuantItem::Id(binder) if binder.kind == Kind::Type => {
                    quantified.insert(binder.kid);
                }
                _ => {
                    return Err(unsupported(
                        span,
                        "dependent call instantiation and constraint discharge",
                    ))
                }
            }
        }
        let mut bindings = BTreeMap::new();
        let mut pairs: Vec<_> = actual
            .iter()
            .enumerate()
            .map(|(i, typ)| (callee.child(Step::Argument(i)), typ.clone()))
            .collect();
        if let Some(expected) = expected {
            pairs.push((callee.child(Step::Result), expected.clone()));
        }
        while let Some((formal, actual)) = pairs.pop() {
            let actual = actual.resolved();
            match &formal.node().kind {
                TypKind::Var(symbol) if quantified.contains(symbol) => {
                    if let Some(bound) = bindings.get(symbol) {
                        self.compatible(bound, &actual, span)?;
                    } else {
                        bindings.insert(*symbol, actual);
                    }
                }
                TypKind::Tuple(items) => {
                    let TypKind::Tuple(found) = &actual.node().kind else {
                        return Err(mismatch(span, &formal, &actual));
                    };
                    if items.len() != found.len() {
                        return Err(arity(span, items.len(), found.len()));
                    }
                    for i in 0..items.len() {
                        pairs.push((formal.child(Step::Tuple(i)), actual.child(Step::Tuple(i))));
                    }
                }
                TypKind::App { name, args } => {
                    let TypKind::App {
                        name: other,
                        args: found,
                    } = &actual.node().kind
                    else {
                        return Err(unsupported(span, "synonym expansion during instantiation"));
                    };
                    if formal.root.interner.resolve(name.symbol)
                        != actual.root.interner.resolve(other.symbol)
                    {
                        return Err(mismatch(span, &formal, &actual));
                    }
                    if args.len() != found.len() {
                        return Err(arity(span, args.len(), found.len()));
                    }
                    for (i, (arg, found)) in args.iter().zip(found).enumerate() {
                        match (&arg.kind, &found.kind) {
                            (TypArgKind::Typ(_), TypArgKind::Typ(_)) => pairs.push((
                                formal.child(Step::TypeArgument(i)),
                                actual.child(Step::TypeArgument(i)),
                            )),
                            _ => {
                                return Err(unsupported(
                                    span,
                                    "numeric or Boolean call argument solving",
                                ))
                            }
                        }
                    }
                }
                _ => self.compatible(&formal, &actual, span)?,
            }
        }
        // Unused quantifiers need no binding. Variables that occur in the
        // result must be fixed before the result can escape this call.
        let mut work = vec![callee.child(Step::Result)];
        while let Some(typ) = work.pop() {
            match &typ.node().kind {
                TypKind::Var(symbol)
                    if quantified.contains(symbol) && !bindings.contains_key(symbol) =>
                {
                    return Err(unsupported(
                        span,
                        "cannot infer polymorphic result variable",
                    ))
                }
                TypKind::Tuple(items) => {
                    work.extend((0..items.len()).map(|i| typ.child(Step::Tuple(i))))
                }
                TypKind::App { args, .. } => {
                    work.extend(args.iter().enumerate().filter_map(|(i, arg)| {
                        matches!(arg.kind, TypArgKind::Typ(_))
                            .then(|| typ.child(Step::TypeArgument(i)))
                    }))
                }
                _ => {}
            }
        }
        callee.substitutions = Arc::new(bindings);
        Ok(callee)
    }
    fn run(&mut self, mut tasks: Vec<Task<'_>>) -> TypeResult<()> {
        while let Some(Task {
            exp,
            expected,
            scope,
            returning,
        }) = tasks.pop()
        {
            self.expressions += 1;
            let expected = expected.resolved();
            match &exp.kind {
                ExpKind::Lit(lit) => {
                    let actual = self.literal(lit)?;
                    self.compatible(&expected, &actual, exp.span)?;
                }
                ExpKind::Id(name) => {
                    let actual = self.id(name, &scope)?;
                    self.compatible(&expected, &actual, exp.span)?;
                }
                ExpKind::If {
                    cond,
                    then_branch,
                    else_branch,
                } => {
                    tasks.push(Task {
                        exp: else_branch,
                        expected: expected.clone(),
                        scope: scope.clone(),
                        returning: returning.clone(),
                    });
                    tasks.push(Task {
                        exp: then_branch,
                        expected,
                        scope: scope.clone(),
                        returning: returning.clone(),
                    });
                    tasks.push(Task {
                        exp: cond,
                        expected: self.builtin("bool"),
                        scope,
                        returning,
                    });
                }
                ExpKind::Block(items) => {
                    if items.is_empty() {
                        let unit = self.builtin("unit");
                        self.compatible(&expected, &unit, exp.span)?;
                    }
                    for (index, item) in items.iter().enumerate().rev() {
                        let expected = if index + 1 == items.len() {
                            expected.clone()
                        } else {
                            self.builtin("unit")
                        };
                        tasks.push(Task {
                            exp: item,
                            expected,
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                }
                ExpKind::Typ { typ, exp: child } => {
                    let annotated = self.annotation(typ)?;
                    self.compatible(&expected, &annotated, exp.span)?;
                    tasks.push(Task {
                        exp: child,
                        expected: annotated,
                        scope,
                        returning,
                    });
                }
                ExpKind::App { func, args } => {
                    let mut callee = self.id(func, &scope)?.resolved();
                    let TypKind::Fn { args: formal, .. } = &callee.node().kind else {
                        return Err(unsupported(exp.span, "application of non-function"));
                    };
                    if args.len() != formal.len() {
                        return Err(arity(exp.span, formal.len(), args.len()));
                    }
                    if !callee.root.typschm.quant.is_empty() {
                        let actual = args
                            .iter()
                            .map(|arg| self.infer(arg, &scope))
                            .collect::<TypeResult<Vec<_>>>()?;
                        callee = self.instantiate(callee, &actual, Some(&expected), exp.span)?;
                    }
                    self.compatible(&expected, &callee.child(Step::Result), exp.span)?;
                    for (index, arg) in args.iter().enumerate().rev() {
                        tasks.push(Task {
                            exp: arg,
                            expected: callee.child(Step::Argument(index)),
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                }
                ExpKind::Tuple(items) => {
                    let TypKind::Tuple(types) = &expected.node().kind else {
                        return Err(unsupported(
                            exp.span,
                            "tuple expression against a type synonym",
                        ));
                    };
                    if items.len() != types.len() {
                        return Err(arity(exp.span, types.len(), items.len()));
                    }
                    for (index, item) in items.iter().enumerate().rev() {
                        tasks.push(Task {
                            exp: item,
                            expected: expected.child(Step::Tuple(index)),
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                }
                ExpKind::Let { pat, bind, body } => {
                    let bound = self.infer(bind, &scope)?;
                    let mut child = Scope {
                        locals: BTreeMap::new(),
                        parent: Some(scope.clone()),
                    };
                    self.pattern(pat, bound.clone(), false, &mut child)?;
                    tasks.push(Task {
                        exp: body,
                        expected,
                        scope: Arc::new(child),
                        returning: returning.clone(),
                    });
                    tasks.push(Task {
                        exp: bind,
                        expected: bound,
                        scope,
                        returning,
                    });
                }
                ExpKind::Return(child) => tasks.push(Task {
                    exp: child,
                    expected: returning.clone(),
                    scope,
                    returning,
                }),
                ExpKind::Undef => {}
                ExpKind::Sizeof(_) => {
                    return Err(unsupported(exp.span, "dependent size expression typing"))
                }
                ExpKind::List(items) if expected.name() == Some("list") => {
                    let TypKind::App { args, .. } = &expected.node().kind else {
                        return Err(unsupported(exp.span, "list type arguments"));
                    };
                    if !matches!(args.first().map(|arg| &arg.kind), Some(TypArgKind::Typ(_))) {
                        return Err(unsupported(exp.span, "list element type"));
                    }
                    for item in items.iter().rev() {
                        tasks.push(Task {
                            exp: item,
                            expected: expected.child(Step::TypeArgument(0)),
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                }
                _ => {
                    return Err(unsupported(
                        exp.span,
                        "expression rule is not yet implemented in the canonical checker",
                    ))
                }
            }
        }
        Ok(())
    }
}
