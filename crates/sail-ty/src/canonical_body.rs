#![forbid(unsafe_code)]
//! Bidirectional checking over the canonical AST using explicit work stacks.
//! Unsupported rules fail; no placeholder or unproved obligation is accepted.

use crate::{
    error::{TypeError, TypeErrorKind, TypeResult},
    program::{Program, Scheme},
};
use sail_ast::*;
use sail_solver::canonical::{Argument, Bindings, CanonicalQuery, Context, View};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

#[derive(Clone, PartialEq, Eq)]
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
    scalars: Arc<ScalarBindings>,
    /// OCaml's implicit_to_int applies to function arguments, not returns.
    implicit_argument: bool,
}
#[derive(Clone)]
struct Scalar {
    owner: Type,
    index: usize,
}
impl Scalar {
    fn argument(&self) -> &TypArg {
        let TypKind::App { args, .. } = &self.owner.node().kind else {
            unreachable!("checked scalar path")
        };
        &args[self.index]
    }
    fn resolved(&self) -> Self {
        let mut scalar = self.clone();
        loop {
            let symbol = match &scalar.argument().kind {
                TypArgKind::NExp(Node {
                    kind: NExpKind::Var(symbol),
                    ..
                }) => *symbol,
                TypArgKind::Bool(c) if matches!(c.kind, ConstraintKind::Var(_)) => {
                    let ConstraintKind::Var(symbol) = c.kind else {
                        unreachable!()
                    };
                    symbol
                }
                _ => break,
            };
            let Some(bound) = scalar.owner.scalars.map.get(&symbol) else {
                break;
            };
            scalar = bound.clone();
        }
        scalar
    }
    fn view(&self) -> Argument<'_> {
        Argument {
            argument: self.argument(),
            context: self.owner.proof_context(),
        }
    }
}
#[derive(Default)]
struct ScalarBindings {
    map: BTreeMap<Symbol, Scalar>,
}
impl Drop for ScalarBindings {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        for (_, mut scalar) in std::mem::take(&mut self.map) {
            pending.push(std::mem::take(&mut scalar.owner.scalars));
        }
        while let Some(bindings) = pending.pop() {
            if let Ok(mut bindings) = Arc::try_unwrap(bindings) {
                for (_, mut scalar) in std::mem::take(&mut bindings.map) {
                    pending.push(std::mem::take(&mut scalar.owner.scalars));
                }
            }
        }
    }
}
static EMPTY_PROOF_VARS: BTreeMap<String, u64> = BTreeMap::new();
impl Bindings for Type {
    fn resolve(&self, symbol: Symbol) -> Option<Argument<'_>> {
        self.scalars.map.get(&symbol).map(Scalar::view)
    }
    fn rigid(&self, name: &str) -> Option<(usize, Kind)> {
        self.scope.typschm.quant.iter().find_map(|q| match q {
            QuantItem::Id(k) if self.scope.interner.resolve(k.kid) == Some(name) => {
                Some((Arc::as_ptr(&self.scope) as usize, k.kind))
            }
            _ => None,
        })
    }
}
impl Type {
    fn proof_context(&self) -> Context<'_> {
        Context {
            interner: &self.root.interner,
            variables: &EMPTY_PROOF_VARS,
            bindings: Some(self),
        }
    }
    fn root(root: Arc<Scheme>) -> Self {
        Self {
            scope: root.clone(),
            root,
            path: vec![],
            substitutions: Arc::default(),
            scalars: Arc::default(),
            implicit_argument: false,
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
        child.implicit_argument = matches!(step, Step::Argument(_));
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
                self.root.interner.resolve(name.symbol).map(|name| {
                    if name == "implicit" && self.implicit_argument {
                        "atom"
                    } else {
                        name
                    }
                })
            }
            _ => None,
        }
    }
    fn label(&self) -> String {
        self.name()
            .map(|name| {
                match name {
                    "atom" => "int",
                    "atom_bool" => "bool",
                    name => name,
                }
                .to_owned()
            })
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
    /// A predicate is kept as a borrowed view of the condition's canonical
    /// type, including its call substitutions and rigid witness identities.
    facts: Vec<(Scalar, bool)>,
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
    fn branch(self: &Arc<Self>, condition: Option<&Type>, negated: bool) -> Arc<Self> {
        match condition.filter(|typ| typ.name() == Some("atom_bool")) {
            Some(condition) => Arc::new(Self {
                locals: BTreeMap::new(),
                facts: vec![(
                    Scalar {
                        owner: condition.clone(),
                        index: 0,
                    },
                    negated,
                )],
                parent: Some(self.clone()),
            }),
            None => self.clone(),
        }
    }
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
    calls: HashMap<*const Exp, Type>,
    proof_scope: Arc<Scope>,
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
        calls: HashMap::new(),
        proof_scope: Arc::default(),
    };
    match &def.kind {
        DefKind::Fundef { rec, funcls, .. } => {
            program.check_function_header(def, &checker.it, position)?;
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
    /// Open a plain scalar value into a fresh rigid singleton witness. Each
    /// immutable binding or expression occurrence keeps its own witness; two
    /// unrelated integers/booleans never share a numeric type variable.
    fn open_value(&self, typ: Type) -> Type {
        let (constructor, kind) = match (&typ.node().kind, typ.name()) {
            (TypKind::Id(_), Some("int")) => ("atom", Kind::Int),
            (TypKind::Id(_), Some("bool")) => ("atom_bool", Kind::Bool),
            _ => return typ,
        };
        let mut interner = SimpleInterner::new();
        let name = Name::dummy(interner.intern(constructor));
        let kid = interner.intern("__value");
        let argument = if kind == Kind::Int {
            TypArgKind::NExp(Node::dummy(NExpKind::Var(kid)))
        } else {
            TypArgKind::Bool(Box::new(Node::dummy(ConstraintKind::Var(kid))))
        };
        Type::root(Arc::new(Scheme {
            typschm: TypSchm {
                quant: vec![QuantItem::Id(KOpt { kind, kid })],
                typ: Node::dummy(TypKind::App {
                    name,
                    args: vec![Node::dummy(argument)],
                }),
            },
            interner: Arc::new(interner),
        }))
    }
    fn value_type(&mut self, exp: &Exp, declared: Type) -> Type {
        if let Some(typ) = self.inferred.get(&(exp as *const Exp)) {
            return typ.clone();
        }
        let typ = self.open_value(declared);
        self.inferred.insert(exp as *const Exp, typ.clone());
        typ
    }
    fn proof_assumptions<'a>(&'a self, scope: &'a Option<Type>) -> Vec<View<'a>> {
        let mut assumptions: Vec<_> = self
            .program
            .constraints(self.position)
            .map(|(constraint, it)| View {
                negated: false,
                constraint,
                context: Context {
                    interner: it,
                    variables: &EMPTY_PROOF_VARS,
                    bindings: None,
                },
            })
            .collect();
        if let Some(scope) = scope {
            assumptions.extend(scope.root.typschm.quant.iter().filter_map(|q| {
                if let QuantItem::Constraint(c) = q {
                    Some(View {
                        negated: false,
                        constraint: c,
                        context: scope.proof_context(),
                    })
                } else {
                    None
                }
            }));
        }
        let mut current = Some(self.proof_scope.as_ref());
        while let Some(scope) = current {
            for (scalar, negated) in &scope.facts {
                let TypArgKind::Bool(constraint) = &scalar.argument().kind else {
                    unreachable!("Boolean branch fact")
                };
                assumptions.push(View {
                    constraint,
                    context: scalar.owner.proof_context(),
                    negated: *negated,
                });
            }
            current = scope.parent.as_deref();
        }
        assumptions
    }
    fn scalar_equal(&self, left: &Scalar, right: &Scalar, span: Span) -> TypeResult<()> {
        let left = left.resolved();
        let right = right.resolved();
        if left.index == right.index
            && left.owner.path == right.owner.path
            && Arc::ptr_eq(&left.owner.root, &right.owner.root)
            && Arc::ptr_eq(&left.owner.scope, &right.owner.scope)
            && Arc::ptr_eq(&left.owner.scalars, &right.owner.scalars)
        {
            return Ok(());
        }
        if left.owner.scalars.map.is_empty()
            && right.owner.scalars.map.is_empty()
            && crate::canonical_eq::argument(
                left.argument(),
                right.argument(),
                &left.owner.root.interner,
                &right.owner.root.interner,
                Arc::ptr_eq(&left.owner.scope, &right.owner.scope),
            )
        {
            return Ok(());
        }
        let scope = self.context.as_ref().map(|s| Type::root(s.clone()));
        let query =
            CanonicalQuery::equality(&self.proof_assumptions(&scope), left.view(), right.view())
                .map_err(|e| unsupported(span, e))?;
        if !self.program.prove(query).is_valid() {
            return Err(TypeError::constraint_not_proved(
                span,
                "dependent type argument equality",
            ));
        }
        Ok(())
    }
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
    fn call_candidates(&self, name: &Name, scope: &Scope) -> TypeResult<Vec<Type>> {
        let name_text = self.name(name)?;
        if scope.get(name_text).is_none() {
            if let Some(candidates) = self.program.overload_candidates(name_text, self.position)? {
                return Ok(candidates.into_iter().map(Type::root).collect());
            }
        }
        Ok(vec![self.id(name, scope)?.resolved()])
    }
    fn select_call(
        &self,
        candidates: Vec<Type>,
        actual: &[Type],
        expected: Option<&Type>,
        span: Span,
    ) -> TypeResult<Type> {
        let single = candidates.len() == 1;
        let mut last = None;
        for callee in candidates {
            let attempt = (|| {
                let TypKind::Fn { args, .. } = &callee.node().kind else {
                    return Err(unsupported(span,"overload candidate is not an elaborated function (mapping expansion may be required)"));
                };
                if args.len() != actual.len() {
                    return Err(arity(span, args.len(), actual.len()));
                }
                let callee = if callee.root.typschm.quant.is_empty() {
                    callee
                } else {
                    self.instantiate(callee, actual, expected, span)?
                };
                for (index, typ) in actual.iter().enumerate() {
                    self.compatible(&callee.child(Step::Argument(index)), typ, span)?;
                }
                if let Some(expected) = expected {
                    self.compatible(expected, &callee.child(Step::Result), span)?;
                }
                Ok(callee)
            })();
            match attempt {
                Ok(callee) => return Ok(callee),
                Err(error) if matches!(error.kind, TypeErrorKind::Unsupported(_)) => {
                    return Err(error)
                }
                Err(error) => last = Some(error),
            }
        }
        if single {
            return Err(last.expect("one attempted candidate"));
        }
        Err(TypeError {
            span,
            kind: TypeErrorKind::QuantError(format!(
                "no matching overload{}",
                last.map(|e| format!(": {e}")).unwrap_or_default()
            )),
        })
    }
    fn literal(&mut self, lit: &Lit) -> TypeResult<Type> {
        Ok(match crate::canonical_literal::classify(lit, &self.it)? {
            crate::canonical_literal::LiteralType::Primitive(name) => self.builtin(name),
            crate::canonical_literal::LiteralType::Refined(name, arg) => self.refined(name, arg),
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
                (TypKind::Id(_), TypKind::Id(_)) => {
                    if expected.name() != found.name() {
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
                (TypKind::App { args: aa, .. }, TypKind::App { args: ba, .. })
                    if expected.name() == found.name() =>
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
                            _ => {
                                self.scalar_equal(
                                    &Scalar {
                                        owner: expected.clone(),
                                        index: i,
                                    },
                                    &Scalar {
                                        owner: found.clone(),
                                        index: i,
                                    },
                                    span,
                                )?;
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
                        let typ = self.open_value(typ);
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
                    let typ = self.open_value(typ);
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
        self.proof_scope = Arc::default();
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
        self.proof_scope = scope.clone();
        let body_scope = if let Some(guard) = guard {
            let condition = self.condition(guard, &scope)?;
            scope.branch(condition.as_ref(), false)
        } else {
            scope.clone()
        };
        let mut tasks = vec![Task {
            exp: body,
            expected: result.clone(),
            scope: body_scope,
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
    fn condition(&mut self, exp: &Exp, scope: &Scope) -> TypeResult<Option<Type>> {
        match self.infer(exp, scope) {
            Ok(typ) => {
                let boolean = self.builtin("bool");
                self.compatible(&boolean, &typ, exp.span)?;
                Ok(Some(typ.resolved()))
            }
            // Some expressions can be checked against bool before they can be
            // synthesized. The ordinary task must still check this condition;
            // it contributes no assumptions until its type can be inferred.
            Err(error) if matches!(error.kind, TypeErrorKind::Unsupported(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn infer(&mut self, exp: &Exp, scope: &Scope) -> TypeResult<Type> {
        enum Synthesis<'a> {
            Enter(&'a Exp),
            Call(&'a Exp, Vec<Type>, usize),
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
                    let declared = match &exp.kind {
                        ExpKind::Lit(lit) => self.literal(lit)?,
                        ExpKind::Id(name) => self.id(name, scope)?,
                        ExpKind::Typ { typ, .. } => self.annotation(typ)?,
                        ExpKind::App { func, args } => {
                            let mut candidates = self.call_candidates(func, scope)?;
                            if candidates.len() != 1 {
                                work.push(Synthesis::Call(exp, candidates, args.len()));
                                work.extend(args.iter().rev().map(Synthesis::Enter));
                                continue;
                            }
                            let callee = candidates.pop().unwrap();
                            let TypKind::Fn { args: formal, .. } = &callee.node().kind else {
                                return Err(unsupported(exp.span, "application of non-function"));
                            };
                            if args.len() != formal.len() {
                                return Err(arity(exp.span, formal.len(), args.len()));
                            }
                            if callee.root.typschm.quant.is_empty() {
                                callee.child(Step::Result).resolved()
                            } else {
                                work.push(Synthesis::Call(exp, vec![callee], args.len()));
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
                    };
                    results.push(self.value_type(exp, declared));
                }
                Synthesis::Call(exp, candidates, count) => {
                    let actual = results.split_off(results.len() - count);
                    let callee = self.select_call(candidates, &actual, None, exp.span)?;
                    let result = self.value_type(exp, callee.child(Step::Result).resolved());
                    self.calls.insert(exp as *const Exp, callee);
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
        let mut quantified_scalars = BTreeMap::new();
        for item in &callee.root.typschm.quant {
            match item {
                QuantItem::Id(binder) if binder.kind == Kind::Type => {
                    quantified.insert(binder.kid);
                }
                QuantItem::Id(binder) => {
                    quantified_scalars.insert(binder.kid, binder.kind);
                }
                QuantItem::Constraint(_) => {}
            }
        }
        let mut bindings = BTreeMap::new();
        let mut scalar_bindings: BTreeMap<Symbol, Scalar> = BTreeMap::new();
        let mut scalar_checks = Vec::new();
        let mut pairs: Vec<_> = actual
            .iter()
            .enumerate()
            .map(|(i, typ)| (callee.child(Step::Argument(i)), typ.clone(), false))
            .collect();
        if let Some(expected) = expected {
            pairs.push((callee.child(Step::Result), expected.clone(), true));
        }
        while let Some((formal, actual, result_context)) = pairs.pop() {
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
                        pairs.push((
                            formal.child(Step::Tuple(i)),
                            actual.child(Step::Tuple(i)),
                            result_context,
                        ));
                    }
                }
                TypKind::App { args, .. } => {
                    if matches!(
                        (&actual.node().kind, actual.name(), formal.name()),
                        (TypKind::Id(_), Some("int"), Some("atom"))
                            | (TypKind::Id(_), Some("bool"), Some("atom_bool"))
                    ) {
                        // A plain expected return type forgets the refinement.
                        // Plain arguments still need existential opening.
                        if !result_context {
                            return Err(unsupported(
                                span,
                                "opening an unrefined integer/Boolean call argument",
                            ));
                        }
                        self.compatible(&actual, &formal, span)?;
                        continue;
                    }
                    let TypKind::App { args: found, .. } = &actual.node().kind else {
                        return Err(unsupported(span, "synonym expansion during instantiation"));
                    };
                    if formal.name() != actual.name() {
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
                                result_context,
                            )),
                            (TypArgKind::NExp(_), TypArgKind::NExp(_))
                            | (TypArgKind::Bool(_), TypArgKind::Bool(_)) => {
                                let formal_arg = Scalar {
                                    owner: formal.clone(),
                                    index: i,
                                };
                                let actual_arg = Scalar {
                                    owner: actual.clone(),
                                    index: i,
                                }
                                .resolved();
                                let variable = match &arg.kind {
                                    TypArgKind::NExp(Node {
                                        kind: NExpKind::Var(k),
                                        ..
                                    }) => Some((*k, Kind::Int)),
                                    TypArgKind::Bool(c) => {
                                        if let ConstraintKind::Var(k) = c.kind {
                                            Some((k, Kind::Bool))
                                        } else {
                                            None
                                        }
                                    }
                                    _ => None,
                                };
                                if let Some((symbol, kind)) =
                                    variable.filter(|(s, k)| quantified_scalars.get(s) == Some(k))
                                {
                                    let _ = kind;
                                    if let Some(bound) = scalar_bindings.get(&symbol) {
                                        scalar_checks.push((bound.clone(), actual_arg, false));
                                    } else {
                                        scalar_bindings.insert(symbol, actual_arg);
                                    }
                                } else {
                                    scalar_checks.push((formal_arg, actual_arg, true));
                                }
                            }
                            _ => return Err(mismatch(span, &formal, &actual)),
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
        if quantified_scalars
            .keys()
            .any(|symbol| !scalar_bindings.contains_key(symbol))
        {
            return Err(unsupported(span,"cannot infer dependent call parameter (existential opening or arithmetic solving required)"));
        }
        callee.scalars = Arc::new(ScalarBindings {
            map: scalar_bindings,
        });
        for (mut formal, actual, instantiate_formal) in scalar_checks {
            if instantiate_formal {
                formal.owner.scalars = callee.scalars.clone();
            }
            self.scalar_equal(&formal, &actual, span)?;
        }
        let scope = self.context.as_ref().map(|s| Type::root(s.clone()));
        let assumptions = self.proof_assumptions(&scope);
        for q in &callee.root.typschm.quant {
            if let QuantItem::Constraint(c) = q {
                let query = CanonicalQuery::implication(
                    &assumptions,
                    View {
                        negated: false,
                        constraint: c,
                        context: callee.proof_context(),
                    },
                    None,
                )
                .map_err(|e| unsupported(span, e))?;
                if !self.program.prove(query).is_valid() {
                    return Err(TypeError::constraint_not_proved(
                        span,
                        "dependent call quantifier constraint",
                    ));
                }
            }
        }
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
            self.proof_scope = scope.clone();
            self.expressions += 1;
            let expected = expected.resolved();
            match &exp.kind {
                ExpKind::Lit(lit) => {
                    let actual = self.literal(lit)?;
                    self.compatible(&expected, &actual, exp.span)?;
                }
                ExpKind::Id(name) => {
                    let actual = self.id(name, &scope)?;
                    let actual = self.value_type(exp, actual);
                    self.compatible(&expected, &actual, exp.span)?;
                }
                ExpKind::If {
                    cond,
                    then_branch,
                    else_branch,
                } => {
                    let condition = self.condition(cond, &scope)?;
                    tasks.push(Task {
                        exp: else_branch,
                        expected: expected.clone(),
                        scope: scope.branch(condition.as_ref(), true),
                        returning: returning.clone(),
                    });
                    tasks.push(Task {
                        exp: then_branch,
                        expected,
                        scope: scope.branch(condition.as_ref(), false),
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
                    let value = self.value_type(exp, annotated.clone());
                    self.compatible(&expected, &value, exp.span)?;
                    tasks.push(Task {
                        exp: child,
                        expected: annotated,
                        scope,
                        returning,
                    });
                }
                ExpKind::App { func, args } => {
                    let mut candidates = self.call_candidates(func, &scope)?;
                    let callee = if let Some(callee) = self.calls.get(&(exp as *const Exp)) {
                        callee.clone()
                    } else if candidates.len() != 1 || !candidates[0].root.typschm.quant.is_empty()
                    {
                        let actual = args
                            .iter()
                            .map(|arg| self.infer(arg, &scope))
                            .collect::<TypeResult<Vec<_>>>()?;
                        self.select_call(candidates, &actual, Some(&expected), exp.span)?
                    } else {
                        candidates.pop().unwrap()
                    };
                    let TypKind::Fn { args: formal, .. } = &callee.node().kind else {
                        return Err(unsupported(exp.span, "application of non-function"));
                    };
                    if args.len() != formal.len() {
                        return Err(arity(exp.span, formal.len(), args.len()));
                    }
                    let result = self.value_type(exp, callee.child(Step::Result).resolved());
                    self.compatible(&expected, &result, exp.span)?;
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
                        facts: Vec::new(),
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
